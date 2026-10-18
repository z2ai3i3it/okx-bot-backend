use std::sync::Arc;

use crate::{
    bot::strategies::base::StrategyAction,
    okx::{rate_limiter::OkxRateLimiter, ws_trade::OkxWsTradeClient},
    pipeline::{
        context::PipelineContext,
        steps::{CemStep, DedupStep, OkxExecutorStep, RateLimitStep},
    },
};

/// สายพานประมวลผลคำสั่งซื้อขาย (Structured Concurrency Order Pipeline)
/// ลำดับขั้นตอน: Dedup -> CEM Gatekeeper -> Rate Limiter -> OKX Executor
#[derive(Clone)]
pub struct OrderPipeline {
    dedup: DedupStep,
    cem: CemStep,
    rate_limit: RateLimitStep,
    executor: OkxExecutorStep,
}

impl OrderPipeline {
    pub fn new(rate_limiter: Arc<OkxRateLimiter>, trade_client: OkxWsTradeClient) -> Self {
        Self {
            dedup: DedupStep::new(),
            cem: CemStep::new(),
            rate_limit: RateLimitStep::new(rate_limiter),
            executor: OkxExecutorStep::new(trade_client),
        }
    }

    pub fn with_cem(
        rate_limiter: Arc<OkxRateLimiter>,
        trade_client: OkxWsTradeClient,
        cem: CemStep,
    ) -> Self {
        Self {
            dedup: DedupStep::new(),
            cem,
            rate_limit: RateLimitStep::new(rate_limiter),
            executor: OkxExecutorStep::new(trade_client),
        }
    }

    pub fn cem_manager(&self) -> Option<Arc<crate::pipeline::steps::CemManager>> {
        self.cem.manager()
    }

    /// ลำเลียงคำสั่งเดี่ยวผ่านแต่ละ Step ใน Pipeline
    pub async fn process_action(
        &self,
        bot_id: &str,
        account_id: &str,
        inst_id: &str,
        action: StrategyAction,
    ) -> Result<(), String> {
        if matches!(action, StrategyAction::Hold) {
            return Ok(());
        }

        let mut ctx = PipelineContext::new(
            bot_id.to_string(),
            account_id.to_string(),
            inst_id.to_string(),
            action,
        )?;

        // Step 1: Dedup
        if let Err(e) = self.dedup.execute(&mut ctx).await {
            // ถูกปฏิเสธตั้งแต่ dedup (เช่น reactivated order ที่ clOrdId ยังไม่ถูกล้างตอน terminal)
            // -> ถ้า CEM เคยจองเงินไว้ (เคส reevaluate จองให้ reactivation) ต้องคืนจองตรงนี้
            //    ไม่งั้น lock ลอย (CEM คิดว่าจองให้ order ที่ไม่เคยถูกวาง)
            self.cem.revert_reservation(&ctx.client_order_id).await;
            return Err(e);
        }

        // Helper macro/closure to clean up dedup if later steps fail
        let run_remaining_steps = async {
            // Step 2: CEM Gatekeeper
            self.cem.execute(&mut ctx).await?;

            // Step 3: Rate Limiter
            self.rate_limit.execute(&mut ctx).await?;

            // 11.5.4 In-Flight Abort Guard: ตรวจสอบระดับ Nanosecond ก่อนส่งคำสั่งลง WebSocket
            // ถ้าออเดอร์ถูก Preempt หรือถูกสั่ง Cancel ตัดหน้าในเสี้ยววินาที ให้ abort ทันที
            if self.cem.is_preempted_or_cancelled(&ctx.client_order_id).await {
                let reason = format!(
                    "Order {} was preempted/cancelled in-flight before socket write — aborting placement",
                    ctx.client_order_id
                );
                log::info!("[Bot #{}] 🚫 [In-Flight Abort] {}", bot_id, reason);
                ctx.reject(reason.clone());
                return Err(reason);
            }

            // Step 4: OKX WS Executor
            self.executor.execute(&mut ctx).await?;

            Ok::<(), String>(())
        };

        if let Err(e) = run_remaining_steps.await {
            self.dedup
                .remove(bot_id, account_id, inst_id, &ctx.client_order_id)
                .await;
            // CEM lock ที่เกิดจาก attempt นี้ (fast-path/reserve-skip) ต้องถูกคืนด้วย
            // ไม่งั้น free/used เพี้ยน จนกว่า order จะมี terminal event จริง (ซึ่งไม่มี เพราะไม่เคยถึง OKX)
            self.cem.revert_reservation(&ctx.client_order_id).await;
            return Err(e);
        }

        // dispatch สำเร็จ -> ล้าง place-failure backoff (ถ้าเคยล้มมาก่อน) กัน retry ถูกหน่วงเกินจำเป็น
        self.cem.note_place_succeeded(&ctx.client_order_id).await;

        log::info!(
            "[Bot #{}] ✨ Order pipeline completed successfully for clOrdId: {}",
            bot_id,
            ctx.client_order_id
        );

        Ok(())
    }

    /// เอา clOrdId ออกจาก dedup cache เมื่อ order ถึง terminal (filled/canceled)
    /// เพื่อให้ CEM สามารถ reactivate/re-place ด้วย clOrdId เดิมได้
    /// (OKX อนุญาตให้ reuse clOrdId หลัง order เข้า terminal state แล้ว)
    /// ระหว่างที่ยังไม่ถึง terminal -> dedup ยังบล็อกการวางซ้ำตามเดิม
    pub async fn clear_dedup(
        &self,
        bot_id: &str,
        account_id: &str,
        inst_id: &str,
        cl_ord_id: &str,
    ) {
        self.dedup.remove(bot_id, account_id, inst_id, cl_ord_id).await;
    }

    /// OKX ปฏิเสธ place "หลัง" dispatch (item ack sCode != 0 เช่น 51008 insufficient balance)
    /// — order ไม่เคยขึ้น exchange และจะไม่มี terminal event มาเอง ต้อง:
    /// 1) ล้าง dedup cache ไม่งั้น clOrdId ตัวนี้ถูก block ตลอดกาล -> CEM reactivate ซ้ำ
    ///    ก็โดน Duplicate ทุกครั้ง -> กลายเป็นลูป 65 รอบ/วินาที (เคสจริง BOT4d81... 03:55:42)
    /// 2) ให้ CEM คืน lock (ไม่เกิด order ผีที่ถือเงินค้าง) + ตั้ง backoff ห้ามปลุกซ้ำทันที
    /// คืน `(asset, amount)` ถ้ามีการคืน lock
    pub async fn handle_place_reject(
        &self,
        bot_id: &str,
        account_id: &str,
        inst_id: &str,
        cl_ord_id: &str,
    ) -> Option<(String, rust_decimal::Decimal)> {
        self.dedup
            .remove(bot_id, account_id, inst_id, cl_ord_id)
            .await;
        self.cem.handle_place_reject(cl_ord_id).await
    }

    /// Cancel ได้ 51400/51603 = order ไม่มีอยู่บน exchange แล้ว (เช่น เคยถูก place-reject ไปก่อน)
    /// -> ล้าง dedup (clOrdId ใช้ซ้ำได้) + ให้ CEM reconcile entry (CancelPending -> Parked + คืน lock,
    /// idempotent: ถ้า terminal event ทำไปแล้วจะเป็น no-op)
    /// คืน `(asset, amount, parked)` ถ้ามีการ reconcile จริง
    pub async fn handle_cancel_gone(
        &self,
        bot_id: &str,
        account_id: &str,
        inst_id: &str,
        cl_ord_id: &str,
    ) -> Option<(String, rust_decimal::Decimal, bool)> {
        self.dedup
            .remove(bot_id, account_id, inst_id, cl_ord_id)
            .await;
        let cem = self.cem_manager()?;
        cem.remove_preemption_cancel(cl_ord_id).await;
        cem.complete_terminal_order(cl_ord_id).await
    }

    /// ลำเลียงชุดคำสั่งหลายคำสั่งผ่าน Pipeline
    pub async fn process_actions(
        &self,
        bot_id: &str,
        account_id: &str,
        inst_id: &str,
        actions: Vec<StrategyAction>,
    ) -> Vec<Result<(), String>> {
        let mut results = Vec::new();
        for action in actions {
            let res = self.process_action(bot_id, account_id, inst_id, action).await;
            results.push(res);
        }
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        okx::{dto::trade::WsPlaceOrderArg, rate_limiter::OkxRateLimiter, ws_trade::OkxWsTradeClient},
        pipeline::steps::{CemManager, CemStep},
    };
    use rust_decimal_macros::dec;

    fn sell_action(cl_ord_id: &str) -> StrategyAction {
        StrategyAction::PlaceOrder {
            arg: WsPlaceOrderArg {
                id: None,
                inst_id: "SUI-USDT".to_string(),
                inst_id_code: None,
                td_mode: "cash".to_string(),
                side: "sell".to_string(),
                ord_type: "post_only".to_string(),
                sz: "10.3665".to_string(),
                px: Some("1.2983".to_string()),
                client_order_id: Some(cl_ord_id.to_string()),
                tag: None,
            },
            reference_bid: None,
            reference_ask: None,
        }
    }

    fn setup() -> (OrderPipeline, Arc<CemManager>, tokio::sync::mpsc::Receiver<crate::okx::ws_trade::WsTradingOp>) {
        let (trade_client, op_rx) = OkxWsTradeClient::new(true);
        let rate_limiter = Arc::new(OkxRateLimiter::new());
        let manager = Arc::new(CemManager::new());
        let pipeline = OrderPipeline::with_cem(
            rate_limiter,
            trade_client,
            CemStep::with_manager(manager.clone()),
        );
        (pipeline, manager, op_rx)
    }

    /// Regression ตาม bug report ข้อ 3B: place ที่ถูกปฏิเสธหลัง CEM จองเงินไว้แล้ว
    /// (เคสจริงจาก zzz.log: reevaluate จอง 10.3665 SUI ให้ reactivated order แล้ว DedupStep block)
    /// ต้องคืน lock เต็มจำนวน และสถานะ CEM ต้องตรงกับ strategy (กลับเป็น Parked เพื่อ retry ได้)
    #[tokio::test]
    async fn failed_place_after_reservation_releases_cem_lock() {
        let (pipeline, manager, _op_rx) = setup();
        manager.update_balance_from_ws("SUI", dec!(20.0), dec!(20.0)).await;

        // 1) place สำเร็จครั้งแรก -> dedup cache มี cl-1, entry Approved, lock 10.3665
        let res = pipeline
            .process_action("bot-1", "acc-1", "SUI-USDT", sell_action("cl-1"))
            .await;
        assert!(res.is_ok(), "place แรกต้องสำเร็จ: {res:?}");
        {
            let balances = manager.balances.lock().await;
            assert_eq!(balances.get("SUI").unwrap().free, dec!(9.6335));
        }

        // 2) เลียนแบบที่ CEM plan mark CancelPending แล้ว order ถึง terminal (preempt cancel)
        //    -> complete_terminal_order เก็บ entry เป็น Parked + คืนเงิน
        let tracked = manager.get_all_tracked_orders().await;
        let mut entry = tracked[0].clone();
        entry.status = crate::pipeline::simulate_cancel_plan::IntentStatus::CancelPending;
        manager.track_order(entry).await;
        let completed = manager.complete_terminal_order("cl-1").await;
        assert!(completed.is_some());
        {
            let balances = manager.balances.lock().await;
            assert_eq!(balances.get("SUI").unwrap().free, dec!(20.0));
        }

        // 3) reevaluate ปลุก + จองเงินใหม่ (เคสจริงจาก zzz.log L150)
        let reactivated = manager.reevaluate_parked_orders().await;
        assert_eq!(reactivated.len(), 1);
        {
            let balances = manager.balances.lock().await;
            assert_eq!(balances.get("SUI").unwrap().free, dec!(9.6335));
        }

        // 4) place ซ้ำ -> DedupStep block (cache ยังมีจากครั้งแรกที่สำเร็จ)
        //    -> pipeline ต้องคืน lock และไม่ทิ้งสถานะ Approved ค้าง
        let res = pipeline
            .process_action("bot-1", "acc-1", "SUI-USDT", sell_action("cl-1"))
            .await;
        assert!(res.is_err(), "place ซ้ำต้องถูก dedup ปฏิเสธ");

        let pool = manager.get_all_tracked_orders().await;
        assert_eq!(
            pool[0].status,
            crate::pipeline::simulate_cancel_plan::IntentStatus::Parked,
            "CEM ต้อง revert เป็น Parked เพื่อให้ retry ได้ (ไม่ใช่ค้าง Approved)"
        );
        {
            let balances = manager.balances.lock().await;
            let sui = balances.get("SUI").unwrap();
            assert_eq!(sui.free, dec!(20.0), "ต้องคืน lock เต็มจำนวน — ห้ามมี lock ลอย");
            assert_eq!(sui.used, dec!(0.0));
        }
    }

    /// Regression ตาม bug report ข้อ 3A + ข้อ 4.1: หลัง order ถึง terminal ต้อง re-place
    /// ด้วย clOrdId เดิมได้ (ล้าง dedup) และ reservation ที่ reevaluate จองไว้แล้ว
    /// ต้องผ่านโดยไม่ lock ซ้ำ (reserve-skip) — เคสจริง sell #3 ใน zzz.log
    #[tokio::test]
    async fn reactivated_place_after_terminal_clear_succeeds_without_double_lock() {
        let (pipeline, manager, _op_rx) = setup();
        manager.update_balance_from_ws("SUI", dec!(20.0), dec!(20.0)).await;

        // 1) place แรกสำเร็จ -> dedup cache มี cl-1
        let res = pipeline
            .process_action("bot-1", "acc-1", "SUI-USDT", sell_action("cl-1"))
            .await;
        assert!(res.is_ok(), "place แรกต้องสำเร็จ: {res:?}");

        // 2) order ถึง terminal (preempt cancel) -> executor ล้าง dedup + เก็บ entry เป็น Parked คืนเงิน
        pipeline.clear_dedup("bot-1", "acc-1", "SUI-USDT", "cl-1").await;
        let tracked = manager.get_all_tracked_orders().await;
        let mut entry = tracked[0].clone();
        entry.status = crate::pipeline::simulate_cancel_plan::IntentStatus::CancelPending;
        manager.track_order(entry).await;
        assert!(manager.complete_terminal_order("cl-1").await.is_some());
        {
            let balances = manager.balances.lock().await;
            assert_eq!(balances.get("SUI").unwrap().free, dec!(20.0));
        }

        // 3) reevaluate ปลุก + จองใหม่
        assert_eq!(manager.reevaluate_parked_orders().await.len(), 1);

        // 4) re-place cl-1 เดิม -> ต้องผ่าน dedup และห้าม lock ซ้ำ
        let res = pipeline
            .process_action("bot-1", "acc-1", "SUI-USDT", sell_action("cl-1"))
            .await;
        assert!(res.is_ok(), "re-place หลัง terminal ต้องผ่าน DedupStep: {res:?}");

        let pool = manager.get_all_tracked_orders().await;
        assert_eq!(
            pool[0].status,
            crate::pipeline::simulate_cancel_plan::IntentStatus::Approved
        );
        {
            let balances = manager.balances.lock().await;
            let sui = balances.get("SUI").unwrap();
            assert_eq!(
                sui.free,
                dec!(9.6335),
                "จองแค่รอบเดียว (reserve-skip) — ห้าม lock ซ้ำจน free หาย"
            );
            assert_eq!(sui.used, dec!(10.3665));
        }
    }

    /// Regression ของ incident app_2026-10-13_03-55 (สมมติฐาน H3 + H2):
    /// place ที่ OKX ปฏิเสธ "หลัง" dispatch (51008 insufficient balance) — เคสจริง BOT4d81...
    /// 1) ต้องคืน lock ของ CEM (ไม่เกิด order ผีที่ถือ 10.3665 SUI ค้างโดยไม่ได้อยู่บน exchange)
    /// 2) ต้องล้าง dedup cache ไม่งั้น clOrdId ถูก block ตลอดกาล (ไม่มี terminal event มาเอง)
    ///    -> CEM reactivate ซ้ำก็โดน Duplicate ทุกครั้ง -> ลูป 65 รอบ/วินาที
    /// 3) re-place ด้วย clOrdId เดิมต้องผ่าน
    #[tokio::test]
    async fn async_place_reject_releases_lock_clears_dedup_and_allows_retry() {
        let (pipeline, manager, _op_rx) = setup();
        manager.update_balance_from_ws("SUI", dec!(20.0), dec!(20.0)).await;

        // 1) place แรกถูก dispatch สำเร็จ -> dedup cache มี cl-1, entry Approved, lock 10.3665
        let res = pipeline
            .process_action("bot-1", "acc-1", "SUI-USDT", sell_action("cl-1"))
            .await;
        assert!(res.is_ok(), "place แรกต้องสำเร็จ: {res:?}");

        // 2) OKX ตอบ 51008 หลัง dispatch (async) -> executor ต้อง reconcile ผ่าน handle_place_reject
        let released = pipeline
            .handle_place_reject("bot-1", "acc-1", "SUI-USDT", "cl-1")
            .await;
        assert_eq!(
            released,
            Some(("SUI".to_string(), dec!(10.3665))),
            "ต้องคืน lock เต็มจำนวน — ห้ามเกิด order ผี"
        );

        // ไม่มี lock ลอย: entry กลับเป็น Parked (รอ retry) และ invariant สะอาด
        let pool = manager.get_all_tracked_orders().await;
        assert_eq!(
            pool[0].status,
            crate::pipeline::simulate_cancel_plan::IntentStatus::Parked
        );
        assert!(manager.invariant_violations().await.is_empty());
        {
            let balances = manager.balances.lock().await;
            let sui = balances.get("SUI").unwrap();
            assert_eq!(sui.free, dec!(20.0));
            assert_eq!(sui.used, dec!(0.0));
        }

        // 3) dedup ถูกล้าง -> re-place ด้วย clOrdId เดิมต้องผ่าน
        let res = pipeline
            .process_action("bot-1", "acc-1", "SUI-USDT", sell_action("cl-1"))
            .await;
        assert!(
            res.is_ok(),
            "re-place หลัง place reject ต้องผ่าน DedupStep: {res:?}"
        );
        assert!(manager.invariant_violations().await.is_empty());
    }

    /// Regression ของสมมติฐาน H2: cancel ได้ 51400/51603 สำหรับ order ที่ไม่เคยขึ้น exchange
    /// -> ต้องล้าง dedup (clOrdId ใช้ซ้ำได้) + CEM reconcile (CancelPending -> Parked + คืน lock)
    /// เพื่อให้ reactivate ครั้งถัดไปผ่าน แทนที่จะถูก Duplicate ตลอดไป
    #[tokio::test]
    async fn cancel_51400_for_order_never_on_exchange_clears_dedup_and_releases_lock() {
        let (pipeline, manager, _op_rx) = setup();
        manager.update_balance_from_ws("SUI", dec!(20.0), dec!(20.0)).await;

        // place ถูก dispatch แล้ว แต่ order ไม่เคย live จริง (place ถูก reject -> ไม่มี terminal event)
        let res = pipeline
            .process_action("bot-1", "acc-1", "SUI-USDT", sell_action("cl-3"))
            .await;
        assert!(res.is_ok(), "place แรกต้องสำเร็จ: {res:?}");

        // CEM plan เลือก order นี้เป็นผู้ถูก preempt -> flip เป็น CancelPending แล้วยิง cancel
        let tracked = manager.get_all_tracked_orders().await;
        let mut entry = tracked[0].clone();
        entry.status = crate::pipeline::simulate_cancel_plan::IntentStatus::CancelPending;
        manager.track_order(entry).await;

        // OKX ตอบ 51400 (order ไม่มีอยู่)
        let reconciled = pipeline
            .handle_cancel_gone("bot-1", "acc-1", "SUI-USDT", "cl-3")
            .await;
        assert_eq!(
            reconciled,
            Some(("SUI".to_string(), dec!(10.3665), true)),
            "ต้องคืน lock และเก็บ entry เป็น Parked รอ reactivation"
        );
        {
            let balances = manager.balances.lock().await;
            let sui = balances.get("SUI").unwrap();
            assert_eq!(sui.free, dec!(20.0));
            assert_eq!(sui.used, dec!(0.0));
        }

        // เรียกซ้ำ -> idempotent (terminal event อีกตัวเห็นก่อนก็ตาม)
        assert!(pipeline
            .handle_cancel_gone("bot-1", "acc-1", "SUI-USDT", "cl-3")
            .await
            .is_none());

        // dedup ถูกล้าง -> re-place ด้วย clOrdId เดิมต้องผ่าน (T5: clOrdId ที่ไม่มีบน exchange ใช้ซ้ำได้)
        let res = pipeline
            .process_action("bot-1", "acc-1", "SUI-USDT", sell_action("cl-3"))
            .await;
        assert!(
            res.is_ok(),
            "re-place หลัง cancel 51400 ต้องผ่าน DedupStep: {res:?}"
        );
    }

    #[tokio::test]
    async fn test_in_flight_abort_does_not_block_reactivated_preempted_order() {
        use crate::pipeline::simulate_cancel_plan::TrackedOrderForCem;
        // จำลองเคสจริง: Order เคยถูก Mark Preemption Cancel
        // แต่เมื่อเงินพอ CEM นำกลับมา Reactivate -> ต้องวางผ่าน ไม่โดน In-Flight Abort บล็อก
        let cem = CemManager::new();
        cem.update_balance_from_ws("SUI", dec!(0.0), dec!(20.0)).await;

        let cl_ord_id = "bot3_sell_preempted";
        let bot_id = "bot-3";

        // 1. Order ถูก Track เป็น Parked
        cem.track_order(TrackedOrderForCem {
            intent_id: cl_ord_id.to_string(),
            bot_id: bot_id.to_string(),
            symbol: "SUI-USDT".to_string(),
            required_asset: "SUI".to_string(),
            locked_amount: dec!(10.0),
            price: dec!(1.5),
            cem_priority: None,
            can_be_canceled: true,
            status: crate::pipeline::simulate_cancel_plan::IntentStatus::Parked,
            created_at: None,
            inst_id_code: None,
            live_confirmed: false,
        }).await;

        // 2. เคยถูก mark preemption cancel ค้างไว้
        cem.mark_preemption_cancel(cl_ord_id).await;
        assert!(cem.is_planned_preemption_cancel(cl_ord_id).await);

        // 3. ปล่อยเงินคืนเข้าระบบ SUI free = 20.0
        cem.update_balance_from_ws("SUI", dec!(20.0), dec!(20.0)).await;

        // 4. reevaluate_parked_orders ปลุกออเดอร์กลับมา
        let reactivated = cem.reevaluate_parked_orders().await;
        assert_eq!(reactivated.len(), 1);
        assert_eq!(reactivated[0].intent_id, cl_ord_id);

        // 5. ตรวจสอบว่า flag planned_preemption_cancels ถูกล้างออกแล้ว
        assert!(!cem.is_planned_preemption_cancel(cl_ord_id).await);
        // และ is_preempted_or_cancelled ต้องคืน false ทันที เพื่อให้ pipeline ยิงผ่าน
        assert!(!cem.is_preempted_or_cancelled(cl_ord_id).await);
    }
}
