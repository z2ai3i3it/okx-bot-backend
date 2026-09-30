use crate::okx::dto::{
    market::OkxTickerData,
    trade::{
        OkxMyTradeData, OkxOrderUpdateData, WsAmendOrderArg, WsCancelOrderArg, WsPlaceOrderArg,
    },
};

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
    /// รับ Ticker update จาก Public WebSocket (ราคา Bid, Ask, Last)
    /// คืนค่า Action ที่ต้องการส่งต่อเข้า Order Pipeline
    fn on_ticker(&mut self, ticker: &OkxTickerData) -> Vec<StrategyAction>;

    /// รับสถานะ Order Lifecycle จาก Private WebSocket (`orders` channel)
    /// เช่น Submit, Partially Filled, Filled, Canceled, Rejected
    fn on_order_update(&mut self, order: &OkxOrderUpdateData) -> Vec<StrategyAction>;

    /// รับข้อมูล Trade Fill จาก Private WebSocket (`my-trades` / `fills` channel)
    /// แจ้งเตือนเมื่อเกิดการจับคู่ซื้อขายจริง
    fn on_fill_update(&mut self, fill: &OkxMyTradeData) -> Vec<StrategyAction>;

    /// Hook สำหรับกู้คืน State (Recovery Reconcile) เมื่อ Bot Start/Restart
    /// รับรายการ Trades ที่ตกค้างหรือเกิดขึ้นระหว่างที่บอทออฟไลน์ เพื่อ Replay State ให้เป็นปัจจุบัน
    fn on_recovery_reconcile(&mut self, _missed_trades: &[OkxMyTradeData]) -> Result<(), String> {
        Ok(())
    }

    /// ดึง Rebalance Snapshot ล่าสุด (ถ้ามี) เพื่อนำไป persist ลง database
    fn get_latest_snapshot(&self) -> Option<crate::bot::math::fixed_ratio_cal::RebalanceRecord> {
        None
    }
}
