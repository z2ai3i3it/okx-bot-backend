use rust_decimal::Decimal;
use std::str::FromStr;

use crate::{
    bot::strategies::base::{Strategy, StrategyAction},
    okx::dto::{
        market::OkxTickerData,
        trade::{OkxMyTradeData, OkxOrderUpdateData, WsPlaceOrderArg},
    },
};

/// FixedRatioStrategy (Mock / Test Phase 2)
/// เจตนายังไม่ใช่การคำนวณสูตรเต็ม แต่เป็นการทดสอบ Flow:
/// 1. รับ Ticker แรกเข้ามา
/// 2. วางคู่ Order (Buy / Sell) โดยคำนวณราคาห่างจาก Bid/Ask +/- 5000 tick (หรือ point offset)
/// 3. พิมพ์ log ระบุ Original Bid/Ask ชัดเจน เพื่อยืนยันว่าราคาตลาดสดมาถึงจริง
pub struct GridStrategy {
    pub bot_id: String,
    pub pair: String,
    pub order_size: String,
    pub tick_offset: Decimal,
    pub inst_id_code: Option<u64>,
    pub has_placed_initial_orders: bool,
    pub active_buy_cl_ord_id: Option<String>,
    pub active_sell_cl_ord_id: Option<String>,
}

impl GridStrategy {
    pub fn new(bot_id: String, pair: String, order_size: String) -> Self {
        Self {
            bot_id,
            pair,
            order_size,
            tick_offset: Decimal::from(5000),
            inst_id_code: None,
            has_placed_initial_orders: false,
            active_buy_cl_ord_id: None,
            active_sell_cl_ord_id: None,
        }
    }

    pub fn with_tick_offset(mut self, offset: Decimal) -> Self {
        self.tick_offset = offset;
        self
    }

    pub fn with_inst_id_code(mut self, code: u64) -> Self {
        self.inst_id_code = Some(code);
        self
    }
}

impl Strategy for GridStrategy {
    fn on_ticker(&mut self, ticker: &OkxTickerData) -> Vec<StrategyAction> {
        // กรองเฉพาะ pair ของตัวเอง
        if ticker.inst_id != self.pair {
            return vec![];
        }

        // หากเคยวาง Order คู่เริ่มต้นไปแล้ว จะข้ามไป (ไม่ log ทุก tick เพื่อป้องกัน CPU/Log flood)
        if self.has_placed_initial_orders {
            return vec![];
        }

        let bid_px_str = &ticker.bid_price;
        let ask_px_str = &ticker.ask_price;

        let _bid_px = match Decimal::from_str(bid_px_str) {
            Ok(p) => p,
            Err(_) => return vec![],
        };
        let _ask_px = match Decimal::from_str(ask_px_str) {
            Ok(p) => p,
            Err(_) => return vec![],
        };

        // กำหนดราคาและขนาดแบบ Fix ค่าเพื่อความปลอดภัยสูงสุดในการทดสอบบัญชีจริง
        let target_buy_px = Decimal::from(50000);
        let target_sell_px = Decimal::from(120000);
        let test_sz = "0.0001".to_string();

        // สร้าง clOrdId ที่เป็น alphanumeric เท่านั้น (ความยาว < 32 ตัวอักษร)
        let clean_bot_id = self
            .bot_id
            .strip_prefix("bot-")
            .unwrap_or(&self.bot_id)
            .replace('-', "");
        let short_id = &clean_bot_id[..clean_bot_id.len().min(8)];
        let ts = chrono::Utc::now().timestamp() % 1_000_000;

        let buy_cl_ord_id = format!("b{}buy{}", short_id, ts);
        let sell_cl_ord_id = format!("b{}sell{}", short_id, ts);

        self.active_buy_cl_ord_id = Some(buy_cl_ord_id.clone());
        self.active_sell_cl_ord_id = Some(sell_cl_ord_id.clone());
        self.has_placed_initial_orders = true;

        log::info!(
            "[Bot #{}] 🎯 FixedRatio Test Flow: Placing initial Buy/Sell pair | Base Market [Bid: {}, Ask: {}] | Offset: {} ticks",
            self.bot_id,
            bid_px_str,
            ask_px_str,
            self.tick_offset
        );
        log::info!(
            "[Bot #{}] 🛒 Calculated Orders -> BUY @ {} (Orig Bid: {}), SELL @ {} (Orig Ask: {})",
            self.bot_id,
            target_buy_px,
            bid_px_str,
            target_sell_px,
            ask_px_str
        );

        let inst_id_code = self.inst_id_code;

        let buy_action = StrategyAction::PlaceOrder {
            arg: WsPlaceOrderArg {
                id: None,
                inst_id: self.pair.clone(),
                inst_id_code,
                td_mode: "cash".to_string(),
                side: "buy".to_string(),
                ord_type: "post_only".to_string(),
                sz: test_sz.clone(),
                px: Some(target_buy_px.to_string()),
                client_order_id: Some(buy_cl_ord_id),
                tag: None,
            },
            reference_bid: Some(bid_px_str.clone()),
            reference_ask: Some(ask_px_str.clone()),
        };

        let sell_action = StrategyAction::PlaceOrder {
            arg: WsPlaceOrderArg {
                id: None,
                inst_id: self.pair.clone(),
                inst_id_code,
                td_mode: "cash".to_string(),
                side: "sell".to_string(),
                ord_type: "post_only".to_string(),
                sz: test_sz,
                px: Some(target_sell_px.to_string()),
                client_order_id: Some(sell_cl_ord_id),
                tag: None,
            },
            reference_bid: Some(bid_px_str.clone()),
            reference_ask: Some(ask_px_str.clone()),
        };

        vec![buy_action, sell_action]
    }

    fn on_order_update(&mut self, order: &OkxOrderUpdateData) -> Vec<StrategyAction> {
        if order.inst_id != self.pair {
            return vec![];
        }

        log::info!(
            "[Bot #{}] 📋 Order Update: clOrdId={} ordId={} side={} state={} px={} sz={} fillSz={}",
            self.bot_id,
            order.client_order_id,
            order.order_id,
            order.side,
            order.state,
            order.px,
            order.sz,
            order.fill_size
        );

        // สามารถเพิ่ม logic จัดการ sibling cancellation เมื่อ state == "filled" ในอนาคต
        vec![]
    }

    fn on_fill_update(&mut self, fill: &OkxMyTradeData) -> Vec<StrategyAction> {
        if fill.inst_id != self.pair {
            return vec![];
        }

        log::info!(
            "[Bot #{}] ⚡ Trade Fill Update: tradeId={} clOrdId={} side={} fillPx={} fillSz={} fee={}{}",
            self.bot_id,
            fill.trade_id,
            fill.client_order_id,
            fill.side,
            fill.fill_price,
            fill.fill_size,
            fill.fee,
            fill.fee_currency
        );

        vec![]
    }
}
