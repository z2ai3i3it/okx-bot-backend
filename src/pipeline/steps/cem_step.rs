use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::{
    bot::strategies::base::StrategyAction,
    okx::rate_limiter::OrderOpType,
    pipeline::{
        context::PipelineContext,
        simulate_cancel_plan::{
            simulate_cancel_plan, MidPrice, OrderIntentForCem, TrackedOrderForCem, DEC_ZERO,
        },
    },
};

/// บันทึกสถานะยอดเงินคงเหลือและ Virtual Lock สำหรับสินทรัพย์หนึ่งๆ
#[derive(Debug, Clone)]
pub struct AssetBalance {
    pub free: Decimal,
    pub used: Decimal,
    pub total: Decimal,
}

impl AssetBalance {
    pub fn new(free: Decimal) -> Self {
        Self {
            free,
            used: DEC_ZERO,
            total: free,
        }
    }
}

/// CEM Core Manager จัดการ Pool เงินและ Order ของทั้ง Account (Shared State ข้าม Bots)
#[derive(Debug, Clone, Default)]
pub struct CemManager {
    /// ยอดคงเหลือแยกตาม Currency (e.g. "USDT", "BTC")
    pub balances: Arc<Mutex<HashMap<String, AssetBalance>>>,
    /// Order ที่อยู่ระหว่างเปิดใช้งาน (Tracked Orders Pool)
    pub tracked_orders: Arc<Mutex<HashMap<String, TrackedOrderForCem>>>,
    /// Mid Prices ปัจจุบันของแต่ละตลาดสำหรับใช้ทำ Market-Aware Cancel Plan
    pub mid_prices: Arc<Mutex<HashMap<String, Decimal>>>,
}

impl CemManager {
    pub fn new() -> Self {
        Self {
            balances: Arc::new(Mutex::new(HashMap::new())),
            tracked_orders: Arc::new(Mutex::new(HashMap::new())),
            mid_prices: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// อัปเดต Balance จาก WebSocket (Ground Truth)
    pub async fn update_balance_from_ws(&self, asset: &str, free: Decimal, total: Decimal) {
        let mut balances = self.balances.lock().await;
        let entry = balances.entry(asset.to_uppercase()).or_insert_with(|| AssetBalance {
            free,
            used: DEC_ZERO,
            total,
        });

        // อัปเดต total และคำนวณ free โดยหัก virtual locked (`used`) ออก
        entry.total = total;
        if entry.used > total {
            entry.used = total;
        }
        entry.free = (total - entry.used).max(DEC_ZERO);
        log::debug!(
            "[CEM] 💰 WS Balance updated for {}: total={}, used={}, effective_free={}",
            asset, entry.total, entry.used, entry.free
        );
    }

    /// อัปเดต Mid Price ของคู่เหรียญ
    pub async fn update_mid_price(&self, pair: &str, price: Decimal) {
        let mut mids = self.mid_prices.lock().await;
        mids.insert(crate::pipeline::simulate_cancel_plan::normalize_symbol(pair), price);
    }

    /// ตรวจสอบและขอล็อกเงินทุน (Virtual Lock)
    pub async fn lock_capital(&self, asset: &str, amount: Decimal) -> bool {
        let mut balances = self.balances.lock().await;
        let entry = balances.entry(asset.to_uppercase()).or_insert_with(|| AssetBalance::new(DEC_ZERO));
        if entry.free >= amount {
            entry.free -= amount;
            entry.used += amount;
            log::info!(
                "[CEM] 🔒 Virtual Lock {} {}: remaining_free={}, used={}",
                amount, asset, entry.free, entry.used
            );
            true
        } else {
            false
        }
    }

    /// คลายล็อกเงินทุน (Virtual Unlock) เมื่อ Fill / Cancel / Reject
    pub async fn release_capital(&self, asset: &str, amount: Decimal) {
        let mut balances = self.balances.lock().await;
        let entry = balances.entry(asset.to_uppercase()).or_insert_with(|| AssetBalance::new(DEC_ZERO));
        let release_amt = amount.min(entry.used);
        entry.used -= release_amt;
        entry.free += release_amt;
        log::info!(
            "[CEM] 🔓 Virtual Unlock {} {}: free={}, used={}",
            release_amt, asset, entry.free, entry.used
        );
    }

    /// บันทึก Order เข้าสู่ Tracked Pool
    pub async fn track_order(&self, order: TrackedOrderForCem) {
        let mut pool = self.tracked_orders.lock().await;
        pool.insert(order.intent_id.clone(), order);
    }

    /// นำ Order ออกจาก Tracked Pool
    pub async fn untrack_order(&self, intent_id: &str) -> Option<TrackedOrderForCem> {
        let mut pool = self.tracked_orders.lock().await;
        pool.remove(intent_id)
    }

    /// ดึงรายการ Tracked Orders ทั้งหมด
    pub async fn get_all_tracked_orders(&self) -> Vec<TrackedOrderForCem> {
        let pool = self.tracked_orders.lock().await;
        pool.values().cloned().collect()
    }
}

/// Step 2: CEM Gatekeeper (Capital Efficiency Manager)
/// - ตรวจสอบความพร้อมของยอดเงินทุน (Balance Gate)
/// - ปล่อยผ่านคำสั่งที่ไม่ต้องใช้ทุนเพิ่ม (Cancel / Hold)
/// - คำนวณ Cancel Plan เมื่อเงินไม่พอ สำหรับ Place Order
#[derive(Clone, Default)]
pub struct CemStep {
    manager: Option<Arc<CemManager>>,
}

impl CemStep {
    pub fn new() -> Self {
        Self { manager: None }
    }

    pub fn with_manager(manager: Arc<CemManager>) -> Self {
        Self {
            manager: Some(manager),
        }
    }

    pub async fn execute(&self, ctx: &mut PipelineContext) -> Result<(), String> {
        // หากยังไม่ได้เสียบ CemManager เข้ามา ให้ทำงานเป็น Pass-through เพื่อ backward-compatibility
        let manager = match &self.manager {
            Some(m) => m,
            None => {
                log::info!(
                    "[Bot #{}] 🛡️ [CEM Gatekeeper] Pass-through (no manager injected) for clOrdId: {}",
                    ctx.bot_id, ctx.client_order_id
                );
                return Ok(());
            }
        };

        // 1. CancelOrder: ปล่อยผ่านเสมอ ไม่ใช้เงินทุนเพิ่ม
        if ctx.op_type == OrderOpType::Cancel {
            log::info!(
                "[Bot #{}] 🛡️ [CEM Gatekeeper] Cancel order approved unconditionally: clOrdId={}",
                ctx.bot_id, ctx.client_order_id
            );
            return Ok(());
        }

        // 2. PlaceOrder: ตรวจสอบและจัดสรรเงินทุน
        if let StrategyAction::PlaceOrder { arg, .. } = &ctx.action {
            let symbol = arg.inst_id.clone();
            let side = arg.side.to_lowercase();
            let price_dec = arg.px.as_deref().unwrap_or("0").parse::<Decimal>().unwrap_or(DEC_ZERO);
            let size_dec = arg.sz.parse::<Decimal>().unwrap_or(DEC_ZERO);

            // แยก required_asset: Buy ใช้ Quote, Sell ใช้ Base
            let (base, quote) = match crate::pipeline::simulate_cancel_plan::split_symbol(&symbol) {
                Some(pair) => pair,
                None => (symbol.clone(), "USDT".to_string()),
            };

            let (required_asset, estimated_cost) = if side == "buy" {
                (quote, (price_dec * size_dec))
            } else {
                (base, size_dec)
            };

            // Fast Path: ตรวจสอบเงินพอหรือไม่
            let current_free = {
                let balances = manager.balances.lock().await;
                balances.get(&required_asset.to_uppercase()).map(|b| b.free).unwrap_or(DEC_ZERO)
            };

            if current_free >= estimated_cost && estimated_cost > DEC_ZERO {
                // เงินพอ -> ล็อกเงินแล้วปล่อยผ่านทันที พร้อมเก็บเข้า Tracked Orders Pool
                manager.lock_capital(&required_asset, estimated_cost).await;
                manager.track_order(TrackedOrderForCem {
                    intent_id: ctx.client_order_id.clone(),
                    symbol: symbol.clone(),
                    required_asset: required_asset.clone(),
                    price: price_dec,
                    locked_amount: estimated_cost,
                    can_be_canceled: true,
                    cem_priority: None,
                }).await;

                log::info!(
                    "[Bot #{}] 🛡️ [CEM Gatekeeper] Fast-path APPROVED for {} (cost: {} {})",
                    ctx.bot_id, ctx.client_order_id, estimated_cost, required_asset
                );
                return Ok(());
            }

            // Slow Path: เงินไม่พอ -> จำลอง Cancel Plan
            log::warn!(
                "[Bot #{}] 🛡️ [CEM Gatekeeper] Deficit detected! Needed: {} {}, Free: {} {}. Simulating Cancel Plan...",
                ctx.bot_id, estimated_cost, required_asset, current_free, required_asset
            );

            let intent = OrderIntentForCem {
                intent_id: ctx.client_order_id.clone(),
                symbol: symbol.clone(),
                required_asset: required_asset.clone(),
                price: price_dec,
                amount: size_dec,
                estimated_cost,
            };

            let pending_orders = manager.get_all_tracked_orders().await;
            let mid_prices: Vec<MidPrice> = {
                let mids = manager.mid_prices.lock().await;
                mids.iter()
                    .map(|(pair, price)| MidPrice::new(pair.clone(), *price))
                    .collect()
            };

            let plan = simulate_cancel_plan(&intent, current_free, &pending_orders, &mid_prices);
            log::info!(
                "[Bot #{}] 🛡️ [CEM Gatekeeper] Cancel Plan result: can_approve={}, to_cancel={:?}, reason={}",
                ctx.bot_id, plan.can_approve_intent, plan.orders_to_cancel, plan.reason
            );

            // บันทึก Plan ลงใน Context เผื่อ Executor หรือ Strategy นำไปใช้งาน
            ctx.cancel_plan = Some(plan.clone());

            if plan.can_approve_intent {
                // แม้จะมี cancel plan แต่ต้องรอ cancel orders ในแผนให้เรียบร้อยก่อนจึงปล่อย place order ได้
                let err_msg = format!(
                    "CEM Gatekeeper requires canceling [{}] orders before approving clOrdId={}",
                    plan.orders_to_cancel.join(", "), ctx.client_order_id
                );
                ctx.reject(err_msg.clone());
                return Err(err_msg);
            } else {
                let err_msg = format!(
                    "CEM Gatekeeper rejected clOrdId={}: Insufficient capital (Deficit cannot be covered)",
                    ctx.client_order_id
                );
                ctx.reject(err_msg.clone());
                return Err(err_msg);
            }
        }

        // 3. AmendOrder: ปล่อยผ่าน (ในอนาคตคำนวณ cost diff)
        log::info!(
            "[Bot #{}] 🛡️ [CEM Gatekeeper] Amend order passed for clOrdId: {}",
            ctx.bot_id, ctx.client_order_id
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::okx::dto::trade::WsPlaceOrderArg;

    #[tokio::test]
    async fn test_cem_step_fast_path_approves_when_balance_sufficient() {
        let manager = Arc::new(CemManager::new());
        manager.update_balance_from_ws("USDT", dec!(1000.0), dec!(1000.0)).await;

        let cem_step = CemStep::with_manager(manager.clone());

        let action = StrategyAction::PlaceOrder {
            arg: WsPlaceOrderArg {
                id: None,
                inst_id: "BTC-USDT".to_string(),
                inst_id_code: None,
                td_mode: "cash".to_string(),
                side: "buy".to_string(),
                ord_type: "limit".to_string(),
                sz: "0.001".to_string(),
                px: Some("50000.0".to_string()),
                client_order_id: Some("test_buy_1".to_string()),
                tag: None,
            },
            reference_bid: None,
            reference_ask: None,
        };

        let mut ctx = PipelineContext::new(
            "bot1".to_string(),
            "acc1".to_string(),
            "BTC-USDT".to_string(),
            action,
        ).unwrap();

        let res = cem_step.execute(&mut ctx).await;
        assert!(res.is_ok());

        // ตรวจสอบว่าเงิน USDT ถูกล็อกไป 50 USDT (0.001 * 50000)
        let balances = manager.balances.lock().await;
        let usdt = balances.get("USDT").unwrap();
        assert_eq!(usdt.free, dec!(950.0));
        assert_eq!(usdt.used, dec!(50.0));
    }

    #[tokio::test]
    async fn test_cem_step_slow_path_generates_cancel_plan_when_deficit() {
        let manager = Arc::new(CemManager::new());
        manager.update_balance_from_ws("USDT", dec!(20.0), dec!(20.0)).await;
        manager.update_mid_price("BTC-USDT", dec!(50000.0)).await;

        // มี order ไกลเปิดอยู่ ล็อก USDT ไว้ 100
        manager.track_order(TrackedOrderForCem {
            intent_id: "far_order_1".to_string(),
            symbol: "BTC-USDT".to_string(),
            required_asset: "USDT".to_string(),
            price: dec!(55000.0),
            locked_amount: dec!(100.0),
            can_be_canceled: true,
            cem_priority: None,
        }).await;

        let cem_step = CemStep::with_manager(manager.clone());

        let action = StrategyAction::PlaceOrder {
            arg: WsPlaceOrderArg {
                id: None,
                inst_id: "BTC-USDT".to_string(),
                inst_id_code: None,
                td_mode: "cash".to_string(),
                side: "buy".to_string(),
                ord_type: "limit".to_string(),
                sz: "0.001".to_string(),
                px: Some("50100.0".to_string()), // cost = 50.1 > free 20
                client_order_id: Some("test_buy_2".to_string()),
                tag: None,
            },
            reference_bid: None,
            reference_ask: None,
        };

        let mut ctx = PipelineContext::new(
            "bot1".to_string(),
            "acc1".to_string(),
            "BTC-USDT".to_string(),
            action,
        ).unwrap();

        let res = cem_step.execute(&mut ctx).await;
        assert!(res.is_err());
        assert!(ctx.is_rejected());
        assert!(ctx.cancel_plan.is_some());

        let plan = ctx.cancel_plan.unwrap();
        assert_eq!(plan.orders_to_cancel, vec!["far_order_1".to_string()]);
    }
}
