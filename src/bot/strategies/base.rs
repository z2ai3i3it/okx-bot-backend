use std::{future::Future, pin::Pin};

use crate::okx::dto::{
    market::OkxTickerData,
    trade::{
        OkxMyTradeData, OkxOrderUpdateData, WsAmendOrderArg, WsCancelOrderArg, WsPlaceOrderArg,
    },
};
use crate::domain::strategy::StrategyState;

pub type StrategyRecoveryFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

/// คำสั่งซื้อขายที่ Strategy ตัดสินใจส่งออกมาหลังจากประเมิน Event
#[derive(Debug, Clone)]
pub enum StrategyAction {
    /// สั่งวาง Order ใหม่ (Place Order)
    /// พร้อมเก็บ meta ราคา reference bid/ask ที่ใช้คำนวณ เพื่อ audit/log
    PlaceOrder {
        arg: WsPlaceOrderArg,
        reference_bid: Option<String>,
        reference_ask: Option<String>,
    },
    /// สั่งยกเลิก Order (Cancel Order)
    CancelOrder(WsCancelOrderArg),
    /// สั่งแก้ไข Order (Amend Order)
    AmendOrder(WsAmendOrderArg),
    /// นิ่งเฉย ไม่ต้องดำเนินการใดๆ
    Hold,
}

/// Interface หลักสำหรับทุก Trading Strategy
/// ออกแบบให้รับ WebSocket Events ทั้ง Public และ Private แล้วคืนคำสั่ง Action ออกมา
pub trait Strategy: Send + Sync {
    fn runtime_state(&self) -> Option<StrategyState> {
        None
    }

    fn restore_runtime_state(&mut self, _state: &StrategyState) {}

    fn pending_cancel_orders(&self) -> Vec<WsCancelOrderArg> {
        Vec::new()
    }

    /// Restore strategy-owned state before live market events can create new orders.
    /// Strategies that do not persist recovery state may keep the default implementation.
    fn recover<'a>(
        &'a mut self,
        missed_trades: &'a [OkxMyTradeData],
    ) -> StrategyRecoveryFuture<'a> {
        Box::pin(async move { self.on_recovery_reconcile(missed_trades) })
    }

    /// รับ Ticker update จาก Public WebSocket (ราคา Bid, Ask, Last)
    /// คืนค่า Action ที่ต้องการส่งต่อเข้า Order Pipeline
    fn on_ticker(&mut self, ticker: &OkxTickerData) -> Vec<StrategyAction>;

    /// รับสถานะ Order Lifecycle จาก Private WebSocket (`orders` channel)
    /// เช่น Submit, Partially Filled, Filled, Canceled, Rejected
    fn on_order_update(&mut self, order: &OkxOrderUpdateData) -> Vec<StrategyAction>;

    /// รับข้อมูล Trade Fill จาก Private WebSocket (`my-trades` / `fills` channel)
    /// แจ้งเตือนเมื่อเกิดการจับคู่ซื้อขายจริง
    fn on_fill_update(&mut self, fill: &OkxMyTradeData) -> Vec<StrategyAction>;

    /// ตรวจสอบว่า `client_order_id` เป็นคำสั่งของ Strategy นี้หรือไม่ (รวมถึงที่ถูก Park ไว้)
    ///
    /// Private WS ของ OKX ส่ง order update ของ **ทุกคำสั่งใน account** ให้ทุก connection
    /// ดังนั้น Executor ใช้ method นี้กรอง ไม่ให้บอทอื่นแตะ preemption flag / CEM pool
    /// ของคำสั่งที่ไม่ใช่ของตัวเอง
    ///
    /// Default = true เพื่อไม่เปลี่ยนพฤติกรรม Strategy ที่ยังไม่ override
    fn owns_client_order_id(&self, _client_order_id: &str) -> bool {
        true
    }

    /// Hook สำหรับกู้คืน State (Recovery Reconcile) เมื่อ Bot Start/Restart
    /// รับรายการ Trades ที่ตกค้างหรือเกิดขึ้นระหว่างที่บอทออฟไลน์ เพื่อ Replay State ให้เป็นปัจจุบัน
    fn on_recovery_reconcile(&mut self, _missed_trades: &[OkxMyTradeData]) -> Result<(), String> {
        Ok(())
    }

    /// Hook เมื่อ CEM ปลุกคำสั่งที่เคยถูก Park ไว้หลังจากมี Balance คืนเข้ามา
    /// ให้ Strategy ตรวจสอบว่าคำสั่งนั้นยังเป็น Order ปัจจุบันที่ต้องการวางหรือไม่ และคืน PlaceOrder Action ออกมา
    fn on_cem_reactivate(
        &mut self,
        _reactivated_order: &crate::pipeline::simulate_cancel_plan::TrackedOrderForCem,
    ) -> Vec<StrategyAction> {
        Vec::new()
    }

    /// Hook เมื่อ Order ถูก CEM สั่ง Cancel ล่วงหน้าเพื่อเคลียร์ Balance ให้ Order ที่ใกล้ตลาดกว่า (Preemption)
    /// ให้ Strategy ย้ายคำสั่งนี้เข้า Parked Pool และไม่ Trigger Healing
    fn on_cem_preemption_canceled(&mut self, _cl_ord_id: &str) {}

    /// Hook เมื่อ Action ถูก Pipeline ปฏิเสธ (เช่น CEM Reject/Park)
    fn on_action_failed(&mut self, _action: &StrategyAction, _reason: &str) {}

    /// Hook เมื่อ OKX ปฏิเสธ place "หลัง" dispatch (เช่น 51008 insufficient balance)
    /// — order ไม่เคยขึ้น exchange และจะไม่มี terminal event มาเอง
    /// ให้ strategy ย้ายคำสั่งกลับเป็น PARKED (CEM จะปลุกใหม่เมื่อเงินพอ) และหยุด chase
    fn on_place_rejected(&mut self, _cl_ord_id: &str, _reason: &str) {}

    /// ดึง Rebalance Snapshot ล่าสุด (ถ้ามี) เพื่อนำไป persist ลง database
    fn get_latest_snapshot(&self) -> Option<crate::bot::math::fixed_ratio_cal::RebalanceRecord> {
        None
    }
}
