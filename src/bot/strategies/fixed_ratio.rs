use std::collections::HashMap;
use std::str::FromStr;
use rust_decimal::Decimal;

use crate::{
    bot::{
        math::fixed_ratio_cal::{
            cal_target_amount_improved, cal_target_price, generate_cl_ord_id,
            save_to_db_filled_rebalance, save_to_db_first_filled_rebalance,
            RebalanceRecord, TargetAnchorOrder,
        },
        strategies::base::{Strategy, StrategyAction},
    },
    domain::{
        order::{Side, TrackedOrder},
        strategy::fixed_ratio_rebalance::FixdRatioRebalanceConfig,
    },
    okx::dto::{
        market::OkxTickerData,
        trade::{OkxMyTradeData, OkxOrderUpdateData, WsCancelOrderArg, WsPlaceOrderArg},
    },
};

/// FixedRatioStrategy (Event-Driven & Reactive)
///
/// สถาปัตยกรรมทำงานตาม Event จาก WebSocket ทั้งหมด (ไม่มี Time-based Polling Loop):
/// 1. `on_ticker`: ดัก Ticker ตัวแรกสำหรับคำนวณและวาง First Order (rebalance_first / rebalance_cal)
/// 2. `on_order_update`: เมื่อคำสั่งหนึ่ง `filled` -> สั่ง Cancel Sibling ทันที และตรวจสอบ Safety Waiter
/// 3. `on_fill_update`: บันทึก Snapshot Rebalance (Trade-Centric) สะสม `cumulative_filled` และเมื่อครบจะเริ่มรอบใหม่
pub struct FixedRatioStrategy {
    pub bot_id: String,
    pub pair: String,
    pub config: FixdRatioRebalanceConfig,

    // Runtime States
    pub snapshot: Option<RebalanceRecord>,
    pub tracked_buy: Option<TrackedOrder>,
    pub tracked_sell: Option<TrackedOrder>,

    /// ปริมาณที่ Fill สะสมแล้วต่อ clOrdId (Trade-Centric Guard)
    pub cumulative_filled: HashMap<String, Decimal>,
    /// รายการ Trade ID ที่เคยประมวลผลแล้ว (Deduplication)
    pub processed_trade_ids: Vec<String>,
    /// ตัวดักรอ (Waiter): เมื่อ Order Closed ก่อน Trade Event มาครบ
    /// Map ord_id -> (expected_filled, cl_ord_id, side)
    pub last_trade_waiters: HashMap<String, (Decimal, String, Side)>,

    /// Flag ป้องกันการ trigger ซ้ำซ้อนตอนกำลังคำนวณรอบใหม่
    pub is_calculating: bool,
    /// Flag บอกว่าเคยวางคำสั่งตั้งต้นไปแล้วหรือยัง
    pub has_initialized: bool,
}

impl FixedRatioStrategy {
    pub fn new(bot_id: String, pair: String, config: FixdRatioRebalanceConfig) -> Self {
        Self {
            bot_id,
            pair,
            config,
            snapshot: None,
            tracked_buy: None,
            tracked_sell: None,
            cumulative_filled: HashMap::new(),
            processed_trade_ids: Vec::new(),
            last_trade_waiters: HashMap::new(),
            is_calculating: false,
            has_initialized: false,
        }
    }

    /// ตรวจสอบว่าคู่เทรดตรงกับของตนเองหรือไม่
    fn is_matching_pair(&self, inst_id: &str) -> bool {
        self.pair == inst_id
    }

    /// ดึง instIdCode สำหรับ OKX V5 Private WebSocket (เช่น BTC-USDT = 3)
    fn get_inst_id_code(&self) -> Option<u64> {
        if self.pair == "BTC-USDT" {
            Some(3)
        } else {
            None
        }
    }

    /// คำนวณและสร้าง Action สำหรับการ Rebalance รอบปกติ (Buy & Sell พร้อมกัน)
    fn calculate_and_place_rebalance(&mut self, base_price: Decimal) -> Vec<StrategyAction> {
        let last = match &self.snapshot {
            Some(s) => s.clone(),
            None => {
                log::warn!("[Bot #{}] ⚠️ No snapshot found for rebalance calculation", self.bot_id);
                return vec![];
            }
        };

        // Guard: ตรวจสอบความพร้อมของทุน
        if last.net_base_balance_b <= Decimal::ZERO || last.quote_value_after_q <= Decimal::ZERO {
            log::warn!(
                "[Bot #{}] ⏸️ ทุนไม่เพียงพอสำหรับคำนวณ Rebalance (Base: {}, Quote: {})",
                self.bot_id,
                last.net_base_balance_b,
                last.quote_value_after_q
            );
            return vec![];
        }

        // คำนวณราคาเป้าหมาย Buy และ Sell
        let target_buy_price = cal_target_price(
            Side::Buy,
            last.price_qb,
            last.net_base_balance_b,
            last.quote_value_after_q,
            self.config.base_value_ratio_target,
            self.config.sell_threshold_ratio_target,
            self.config.buy_threshold_ratio_target,
            self.config.min_size_b,
            self.config.decimal_place_price_qb,
            Decimal::ZERO,
            Decimal::ZERO,
        );

        let target_sell_price = cal_target_price(
            Side::Sell,
            last.price_qb,
            last.net_base_balance_b,
            last.quote_value_after_q,
            self.config.base_value_ratio_target,
            self.config.sell_threshold_ratio_target,
            self.config.buy_threshold_ratio_target,
            self.config.min_size_b,
            self.config.decimal_place_price_qb,
            Decimal::ZERO,
            Decimal::ZERO,
        );

        let (buy_price, sell_price) = match (target_buy_price, target_sell_price) {
            (Some(b), Some(s)) => (b, s),
            _ => {
                log::warn!("[Bot #{}] ⚠️ ไม่สามารถคำนวณราคาเป้าหมายได้", self.bot_id);
                return vec![];
            }
        };

        // คำนวณขนาด Order
        let amount_buy = cal_target_amount_improved(
            Side::Buy,
            buy_price,
            last.net_base_balance_b,
            last.quote_value_after_q,
            self.config.base_value_ratio_target,
            self.config.sell_threshold_ratio_target,
            self.config.buy_threshold_ratio_target,
            self.config.min_size_b,
            self.config.decimal_place_amount_b,
            self.config.maker_fee,
            Decimal::ZERO,
            Decimal::ZERO,
        );

        let amount_sell = cal_target_amount_improved(
            Side::Sell,
            sell_price,
            last.net_base_balance_b,
            last.quote_value_after_q,
            self.config.base_value_ratio_target,
            self.config.sell_threshold_ratio_target,
            self.config.buy_threshold_ratio_target,
            self.config.min_size_b,
            self.config.decimal_place_amount_b,
            self.config.maker_fee,
            Decimal::ZERO,
            Decimal::ZERO,
        );

        log::info!(
            "[Bot #{}] 🎯 Rebalance Targets: BUY {} @ {} | SELL {} @ {} (Last Price: {})",
            self.bot_id,
            amount_buy,
            buy_price,
            amount_sell.abs(),
            sell_price,
            base_price
        );

        // อัปเดต Target Anchor ลงใน Snapshot ปัจจุบัน
        let mut updated_snapshot = last.clone();
        updated_snapshot.target_anchor_buy = Some(TargetAnchorOrder {
            side: Side::Buy,
            price_qb: buy_price,
            amount_b: amount_buy.abs(),
        });
        updated_snapshot.target_anchor_sell = Some(TargetAnchorOrder {
            side: Side::Sell,
            price_qb: sell_price,
            amount_b: amount_sell.abs(),
        });
        self.snapshot = Some(updated_snapshot);

        let mut actions = Vec::new();
        let inst_id_code = self.get_inst_id_code();

        // สร้าง Buy Order Action
        if amount_buy.abs() >= self.config.min_size_b {
            let cl_ord_id = generate_cl_ord_id();
            self.tracked_buy = Some(TrackedOrder {
                cl_ord_id: cl_ord_id.clone(),
                side: Side::Buy,
                price: buy_price,
                amount: amount_buy.abs(),
            });

            actions.push(StrategyAction::PlaceOrder {
                arg: WsPlaceOrderArg {
                    id: None,
                    inst_id: self.pair.clone(),
                    inst_id_code,
                    td_mode: "cash".to_string(),
                    side: "buy".to_string(),
                    ord_type: "post_only".to_string(),
                    sz: amount_buy.abs().to_string(),
                    px: Some(buy_price.to_string()),
                    client_order_id: Some(cl_ord_id),
                    tag: None,
                },
                reference_bid: Some(base_price.to_string()),
                reference_ask: None,
            });
        }

        // สร้าง Sell Order Action
        if amount_sell.abs() >= self.config.min_size_b {
            let cl_ord_id = generate_cl_ord_id();
            self.tracked_sell = Some(TrackedOrder {
                cl_ord_id: cl_ord_id.clone(),
                side: Side::Sell,
                price: sell_price,
                amount: amount_sell.abs(),
            });

            actions.push(StrategyAction::PlaceOrder {
                arg: WsPlaceOrderArg {
                    id: None,
                    inst_id: self.pair.clone(),
                    inst_id_code,
                    td_mode: "cash".to_string(),
                    side: "sell".to_string(),
                    ord_type: "post_only".to_string(),
                    sz: amount_sell.abs().to_string(),
                    px: Some(sell_price.to_string()),
                    client_order_id: Some(cl_ord_id),
                    tag: None,
                },
                reference_bid: None,
                reference_ask: Some(base_price.to_string()),
            });
        }

        actions
    }

    /// ตรวจสอบและ Finalize Order เมื่อ Trade Events ครบถ้วน
    fn try_finalize_order(&mut self, cl_ord_id: &str, order_id: &str) -> Vec<StrategyAction> {
        let is_buy = self.tracked_buy.as_ref().map_or(false, |o| o.cl_ord_id == cl_ord_id);
        let is_sell = self.tracked_sell.as_ref().map_or(false, |o| o.cl_ord_id == cl_ord_id);

        if is_buy {
            self.tracked_buy = None;
        }
        if is_sell {
            self.tracked_sell = None;
        }

        self.cumulative_filled.remove(cl_ord_id);
        self.last_trade_waiters.remove(order_id);

        log::info!(
            "[Bot #{}] ✅ Order {} ({}) finalized & resolved",
            self.bot_id,
            cl_ord_id,
            order_id
        );

        // เมื่อทั้งสองฝั่งไม่มีออเดอร์ค้างอยู่ (Clean Slate) -> Trigger Rebalance ทันที
        if self.tracked_buy.is_none() && self.tracked_sell.is_none() {
            let last_px = self.snapshot.as_ref().map_or(Decimal::ZERO, |s| s.price_qb);
            log::info!(
                "[Bot #{}] 🔄 [Session-End] All orders resolved! Triggering next rebalance...",
                self.bot_id
            );
            return self.calculate_and_place_rebalance(last_px);
        }

        vec![]
    }
}

impl Strategy for FixedRatioStrategy {
    fn on_ticker(&mut self, ticker: &OkxTickerData) -> Vec<StrategyAction> {
        if !self.is_matching_pair(&ticker.inst_id) {
            return vec![];
        }

        // Ticker ใช้เฉพาะจังหวะเริ่มต้น (First-Snap) หรือตอนที่ไม่มีออเดอร์และยังไม่มี Snapshot
        if self.has_initialized || self.tracked_buy.is_some() || self.tracked_sell.is_some() {
            return vec![];
        }

        let current_price = match Decimal::from_str(&ticker.last) {
            Ok(p) if p > Decimal::ZERO => p,
            _ => return vec![],
        };

        log::info!(
            "[Bot #{}] 🎬 FixedRatio on_ticker: Starting First Rebalance Evaluation @ Price {}",
            self.bot_id,
            current_price
        );
        self.has_initialized = true;

        // คำนวณ Portfolio เริ่มต้น
        let offset = self.config.decimal_place_price_qb * Decimal::from(100);
        let base_value_q = self.config.initial_capital_b * current_price;
        let side = if self.config.initial_capital_q > base_value_q {
            Side::Buy
        } else {
            Side::Sell
        };

        let adjusted_price = match side {
            Side::Buy => current_price - offset,
            Side::Sell => current_price + offset,
        };

        let amount = cal_target_amount_improved(
            side,
            current_price,
            self.config.initial_capital_b,
            self.config.initial_capital_q,
            self.config.base_value_ratio_target,
            self.config.sell_threshold_ratio_target,
            self.config.buy_threshold_ratio_target,
            self.config.min_size_b,
            self.config.decimal_place_amount_b,
            self.config.maker_fee,
            Decimal::ZERO,
            Decimal::ZERO,
        );

        // กรณีที่ 1: พอร์ตมีความสมดุลอยู่แล้วตั้งแต่แรก (Amount == 0 หรือ < min_size_b)
        if amount == Decimal::ZERO || amount.abs() < self.config.min_size_b {
            log::info!(
                "[Bot #{}] ⚖️ Port is already balanced (Amount: {} < min_size). Seeding initial record...",
                self.bot_id,
                amount
            );

            let record = save_to_db_first_filled_rebalance(
                "limit",
                side.as_str(),
                &self.pair,
                "maker",
                self.config.initial_capital_b,
                self.config.initial_capital_q,
                current_price,
                Decimal::ZERO,
                self.config.maker_fee,
                None,
                None,
            );
            self.snapshot = Some(record);

            // เริ่มคำนวณและวางคำสั่งซื้อขายทั้ง 2 ฝั่งทันที
            return self.calculate_and_place_rebalance(current_price);
        }

        // กรณีที่ 2: พอร์ต Imbalanced -> วาง Order เดี่ยว 1 ฝั่งเพื่อปรับสมดุล
        let cl_ord_id = generate_cl_ord_id();
        let inst_id_code = self.get_inst_id_code();

        let tracked = TrackedOrder {
            cl_ord_id: cl_ord_id.clone(),
            side,
            price: adjusted_price,
            amount: amount.abs(),
        };

        match side {
            Side::Buy => self.tracked_buy = Some(tracked),
            Side::Sell => self.tracked_sell = Some(tracked),
        }

        log::info!(
            "[Bot #{}] 🚀 Placing First Single Order: Side={} Amount={} Price={} (clOrdId={})",
            self.bot_id,
            side.as_str(),
            amount.abs(),
            adjusted_price,
            cl_ord_id
        );

        vec![StrategyAction::PlaceOrder {
            arg: WsPlaceOrderArg {
                id: None,
                inst_id: self.pair.clone(),
                inst_id_code,
                td_mode: "cash".to_string(),
                side: side.as_str().to_string(),
                ord_type: "post_only".to_string(),
                sz: amount.abs().to_string(),
                px: Some(adjusted_price.to_string()),
                client_order_id: Some(cl_ord_id),
                tag: None,
            },
            reference_bid: Some(ticker.bid_price.clone()),
            reference_ask: Some(ticker.ask_price.clone()),
        }]
    }

    fn on_order_update(&mut self, order: &OkxOrderUpdateData) -> Vec<StrategyAction> {
        if !self.is_matching_pair(&order.inst_id) {
            return vec![];
        }

        let cl_ord_id = &order.client_order_id;
        let ord_id = &order.order_id;
        let state = order.state.as_str();

        log::info!(
            "[Bot #{}] 📋 Order Update: clOrdId={} ordId={} side={} state={} fillSz={}",
            self.bot_id,
            cl_ord_id,
            ord_id,
            order.side,
            state,
            order.fill_size
        );

        let mut actions = Vec::new();

        match state {
            "filled" => {
                // 1. สั่ง Cancel Sibling ทันที (ยกเลิกออเดอร์ฝั่งตรงข้าม)
                let inst_id_code = self.get_inst_id_code();
                if let Some(buy_order) = &self.tracked_buy {
                    if &buy_order.cl_ord_id != cl_ord_id {
                        log::info!(
                            "[Bot #{}] 🛑 Filled Trigger: Cancelling sibling BUY order {}",
                            self.bot_id,
                            buy_order.cl_ord_id
                        );
                        actions.push(StrategyAction::CancelOrder(WsCancelOrderArg {
                            inst_id: self.pair.clone(),
                            inst_id_code,
                            order_id: None,
                            client_order_id: Some(buy_order.cl_ord_id.clone()),
                        }));
                    }
                }
                if let Some(sell_order) = &self.tracked_sell {
                    if &sell_order.cl_ord_id != cl_ord_id {
                        log::info!(
                            "[Bot #{}] 🛑 Filled Trigger: Cancelling sibling SELL order {}",
                            self.bot_id,
                            sell_order.cl_ord_id
                        );
                        actions.push(StrategyAction::CancelOrder(WsCancelOrderArg {
                            inst_id: self.pair.clone(),
                            inst_id_code,
                            order_id: None,
                            client_order_id: Some(sell_order.cl_ord_id.clone()),
                        }));
                    }
                }

                // 2. Safety Gate: ตรวจสอบว่า Trade events มาครบตาม fill_size หรือยัง
                let expected_filled = Decimal::from_str(&order.fill_size).unwrap_or(Decimal::ZERO);
                let current_cumulative = self.cumulative_filled.get(cl_ord_id).cloned().unwrap_or(Decimal::ZERO);

                if current_cumulative >= expected_filled && expected_filled > Decimal::ZERO {
                    log::info!(
                        "[Bot #{}] ✅ All trades received for {}. Finalizing order...",
                        self.bot_id,
                        cl_ord_id
                    );
                    let finalize_actions = self.try_finalize_order(cl_ord_id, ord_id);
                    actions.extend(finalize_actions);
                } else {
                    // Trades ยังมาไม่ครบ -> ลง Waiter รอให้ on_fill_update มาปิด
                    let side = if order.side.to_lowercase() == "buy" { Side::Buy } else { Side::Sell };
                    self.last_trade_waiters.insert(ord_id.clone(), (expected_filled, cl_ord_id.clone(), side));
                    log::info!(
                        "[Bot #{}] ⏳ [Order Closed] Waiting for trade fills: {} (Cumulative: {}, Expected: {})",
                        self.bot_id,
                        cl_ord_id,
                        current_cumulative,
                        expected_filled
                    );
                }
            }
            "canceled" => {
                // ตรวจสอบว่ามีการ Fill บางส่วนหรือไม่
                let fill_sz = Decimal::from_str(&order.fill_size).unwrap_or(Decimal::ZERO);
                let current_cumulative = self.cumulative_filled.get(cl_ord_id).cloned().unwrap_or(Decimal::ZERO);

                if fill_sz > Decimal::ZERO && current_cumulative < fill_sz {
                    let side = if order.side.to_lowercase() == "buy" { Side::Buy } else { Side::Sell };
                    self.last_trade_waiters.insert(ord_id.clone(), (fill_sz, cl_ord_id.clone(), side));
                    log::info!(
                        "[Bot #{}] ⏳ [Canceled Partial] Waiting for trades: {} (Cumulative: {}, Expected: {})",
                        self.bot_id,
                        cl_ord_id,
                        current_cumulative,
                        fill_sz
                    );
                } else {
                    let finalize_actions = self.try_finalize_order(cl_ord_id, ord_id);
                    actions.extend(finalize_actions);
                }
            }
            _ => {}
        }

        actions
    }

    fn on_fill_update(&mut self, fill: &OkxMyTradeData) -> Vec<StrategyAction> {
        if !self.is_matching_pair(&fill.inst_id) {
            return vec![];
        }

        let trade_id = &fill.trade_id;
        let ord_id = &fill.order_id;
        let cl_ord_id = &fill.client_order_id;

        // Guard: Deduplication
        if self.processed_trade_ids.contains(trade_id) {
            log::info!("[Bot #{}] ⏭️ Skipping already processed trade: {}", self.bot_id, trade_id);
            return vec![];
        }
        self.processed_trade_ids.push(trade_id.clone());
        if self.processed_trade_ids.len() > 100 {
            self.processed_trade_ids.remove(0);
        }

        let fill_px = Decimal::from_str(&fill.fill_price).unwrap_or(Decimal::ZERO);
        let fill_sz = Decimal::from_str(&fill.fill_size).unwrap_or(Decimal::ZERO);
        let side_str = fill.side.to_lowercase();
        let side = if side_str == "buy" { Side::Buy } else { Side::Sell };

        // 1. สะสม cumulative_filled
        let current_cum = self.cumulative_filled.entry(cl_ord_id.clone()).or_insert(Decimal::ZERO);
        *current_cum += fill_sz;
        let total_cum = *current_cum;

        log::info!(
            "[Bot #{}] ⚡ [Trade-Centric] Fill Event: clOrdId={} side={} px={} sz={} (Cumulative: {})",
            self.bot_id,
            cl_ord_id,
            side_str,
            fill_px,
            fill_sz,
            total_cum
        );

        // 2. คำนวณ Fee และอัปเดต Snapshot
        let fee_rate = self.config.maker_fee;
        let signed_amount = if side == Side::Sell { -fill_sz } else { fill_sz };

        let new_snapshot = match &self.snapshot {
            None => {
                // First Order Fill
                save_to_db_first_filled_rebalance(
                    "limit",
                    side.as_str(),
                    &self.pair,
                    "maker",
                    self.config.initial_capital_b,
                    self.config.initial_capital_q,
                    fill_px,
                    signed_amount,
                    fee_rate,
                    None,
                    None,
                )
            }
            Some(last) => {
                save_to_db_filled_rebalance(
                    "limit",
                    side.as_str(),
                    &self.pair,
                    "maker",
                    last.sequence + 1,
                    last.net_base_balance_b,
                    fill_px,
                    signed_amount,
                    fee_rate,
                    last.quote_value_after_q,
                    last.base_additional_b,
                    last.quote_additional_q,
                    last.target_anchor_buy.clone(),
                    last.target_anchor_sell.clone(),
                )
            }
        };

        log::info!(
            "[Bot #{}] 💾 Snapshot updated: Seq={} NetBase={} QuoteVal={} PortVal={}",
            self.bot_id,
            new_snapshot.sequence,
            new_snapshot.net_base_balance_b,
            new_snapshot.quote_value_after_q,
            new_snapshot.port_value_after_q
        );
        self.snapshot = Some(new_snapshot);

        // 3. ตรวจสอบ Waiter: หากมี Order ที่ Closed รอดักอยู่แล้ว และ Trade สะสมครบตามที่คาดหวัง
        if let Some((expected_sz, target_cl_ord_id, _)) = self.last_trade_waiters.get(ord_id) {
            if total_cum >= *expected_sz {
                log::info!(
                    "[Bot #{}] ✅ Waiter satisfied for {} (Accumulated {} >= Expected {}). Finalizing...",
                    self.bot_id,
                    target_cl_ord_id,
                    total_cum,
                    expected_sz
                );
                return self.try_finalize_order(cl_ord_id, ord_id);
            }
        }

        vec![]
    }

    fn on_recovery_reconcile(&mut self, missed_trades: &[OkxMyTradeData]) -> Result<(), String> {
        if missed_trades.is_empty() {
            log::info!(
                "[Bot #{}] 🔄 [FixedRatio Reconcile] No missed trades to reconcile.",
                self.bot_id
            );
            return Ok(());
        }

        log::info!(
            "[Bot #{}] 🔄 [FixedRatio Reconcile] Replaying {} missed trade(s)...",
            self.bot_id,
            missed_trades.len()
        );

        // 1. กรองเฉพาะ Trade ของ pair บอทตัวนี้ และเรียงตาม timestamp เก่า -> ใหม่
        let mut sorted_trades: Vec<OkxMyTradeData> = missed_trades
            .iter()
            .filter(|t| self.is_matching_pair(&t.inst_id))
            .cloned()
            .collect();

        sorted_trades.sort_by(|a, b| {
            let ts_a = a.ts.parse::<u64>().unwrap_or(0);
            let ts_b = b.ts.parse::<u64>().unwrap_or(0);
            ts_a.cmp(&ts_b)
        });

        // 2. Replay ทีละ Trade
        for trade in &sorted_trades {
            // ข้ามถ้าเคยประมวลผลไปแล้ว
            if self.processed_trade_ids.contains(&trade.trade_id) {
                continue;
            }

            let fill_px = Decimal::from_str(&trade.fill_price).unwrap_or(Decimal::ZERO);
            let fill_sz = Decimal::from_str(&trade.fill_size).unwrap_or(Decimal::ZERO);
            let side_str = trade.side.to_lowercase();
            let side = if side_str == "buy" { Side::Buy } else { Side::Sell };
            let fee_rate = self.config.maker_fee;
            let signed_amount = if side == Side::Sell { -fill_sz } else { fill_sz };

            let new_snapshot = match &self.snapshot {
                None => {
                    save_to_db_first_filled_rebalance(
                        "limit",
                        side.as_str(),
                        &self.pair,
                        "maker",
                        self.config.initial_capital_b,
                        self.config.initial_capital_q,
                        fill_px,
                        signed_amount,
                        fee_rate,
                        None,
                        None,
                    )
                }
                Some(last) => {
                    save_to_db_filled_rebalance(
                        "limit",
                        side.as_str(),
                        &self.pair,
                        "maker",
                        last.sequence + 1,
                        last.net_base_balance_b,
                        fill_px,
                        signed_amount,
                        fee_rate,
                        last.quote_value_after_q,
                        last.base_additional_b,
                        last.quote_additional_q,
                        last.target_anchor_buy.clone(),
                        last.target_anchor_sell.clone(),
                    )
                }
            };

            self.processed_trade_ids.push(trade.trade_id.clone());
            self.snapshot = Some(new_snapshot);
        }

        // 3. Clear tracked active orders และ runtime waiters เพราะ recovery ทำการ clean slate บน exchange แล้ว
        self.tracked_buy = None;
        self.tracked_sell = None;
        self.last_trade_waiters.clear();
        self.cumulative_filled.clear();
        self.is_calculating = false;

        if let Some(snap) = &self.snapshot {
            log::info!(
                "[Bot #{}] ✅ [FixedRatio Reconcile] Completed replay. Active Snapshot: Seq={} NetBase={} QuoteVal={} PortVal={}",
                self.bot_id,
                snap.sequence,
                snap.net_base_balance_b,
                snap.quote_value_after_q,
                snap.port_value_after_q
            );
        }

        Ok(())
    }

    fn get_latest_snapshot(&self) -> Option<RebalanceRecord> {
        self.snapshot.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn mock_config() -> FixdRatioRebalanceConfig {
        FixdRatioRebalanceConfig {
            initial_capital_b: dec!(0.1),
            initial_capital_q: dec!(5000.0),
            base_value_ratio_target: dec!(0.5),
            sell_threshold_ratio_target: dec!(0.02),
            buy_threshold_ratio_target: dec!(0.02),
            maker_fee: dec!(0.0008),
            taker_fee: dec!(0.001),
            min_size_b: dec!(0.0001),
            decimal_place_price_qb: dec!(0.1),
            decimal_place_amount_b: dec!(0.0001),
            decimal_place_total_q: dec!(0.01),
        }
    }

    #[test]
    fn test_event_driven_fixed_ratio_first_order_imbalanced() {
        let config = mock_config();
        let mut strategy = FixedRatioStrategy::new(
            "bot-test-1".to_string(),
            "BTC-USDT".to_string(),
            config,
        );

        // ส่ง Ticker ครั้งแรกเข้ามา (ราคา 60,000 -> base_value = 6,000 > quote_value 5,000 -> ต้อง Sell)
        let ticker = OkxTickerData {
            inst_type: "SPOT".to_string(),
            inst_id: "BTC-USDT".to_string(),
            last: "60000.0".to_string(),
            last_size: "0.1".to_string(),
            ask_price: "60001.0".to_string(),
            ask_size: "1.0".to_string(),
            bid_price: "59999.0".to_string(),
            bid_size: "1.0".to_string(),
            high24h: "61000.0".to_string(),
            low24h: "59000.0".to_string(),
            vol24h: "100.0".to_string(),
            ts: "1700000000".to_string(),
        };

        let actions = strategy.on_ticker(&ticker);
        assert_eq!(actions.len(), 1, "ควรสร้าง First Single Order 1 รายการเพื่อปรับสมดุล");

        if let StrategyAction::PlaceOrder { arg, .. } = &actions[0] {
            assert_eq!(arg.side, "sell");
            assert_eq!(arg.ord_type, "post_only");
            assert_eq!(arg.inst_id_code, Some(3));
        } else {
            panic!("Action should be PlaceOrder");
        }

        assert!(strategy.has_initialized);
        assert!(strategy.tracked_sell.is_some());
        assert!(strategy.tracked_buy.is_none());
    }

    #[test]
    fn test_event_driven_fixed_ratio_first_order_balanced_seeds_and_places_both() {
        let mut config = mock_config();
        // ปรับให้ initial_capital_b * 50,000 = 5,000 USDT เท่ากับ quote_capital พอดี (Balanced)
        config.initial_capital_b = dec!(0.1);
        config.initial_capital_q = dec!(5000.0);

        let mut strategy = FixedRatioStrategy::new(
            "bot-test-2".to_string(),
            "BTC-USDT".to_string(),
            config,
        );

        let ticker = OkxTickerData {
            inst_type: "SPOT".to_string(),
            inst_id: "BTC-USDT".to_string(),
            last: "50000.0".to_string(),
            last_size: "0.1".to_string(),
            ask_price: "50001.0".to_string(),
            ask_size: "1.0".to_string(),
            bid_price: "49999.0".to_string(),
            bid_size: "1.0".to_string(),
            high24h: "51000.0".to_string(),
            low24h: "49000.0".to_string(),
            vol24h: "100.0".to_string(),
            ts: "1700000000".to_string(),
        };

        let actions = strategy.on_ticker(&ticker);
        assert_eq!(actions.len(), 2, "พอร์ตสมดุลอยู่แล้ว ต้องสร้างคำสั่ง Buy และ Sell ทั้งคู่");
        assert!(strategy.snapshot.is_some(), "ต้อง seed snapshot แรกเรียบร้อย");
        assert!(strategy.tracked_buy.is_some());
        assert!(strategy.tracked_sell.is_some());
    }

    #[test]
    fn test_sibling_cancellation_on_filled() {
        let config = mock_config();
        let mut strategy = FixedRatioStrategy::new(
            "bot-test-3".to_string(),
            "BTC-USDT".to_string(),
            config,
        );

        // กำหนด State จำลองว่ากำลังเปิดอยู่ทั้ง Buy และ Sell
        strategy.tracked_buy = Some(TrackedOrder {
            cl_ord_id: "buy123".to_string(),
            side: Side::Buy,
            price: dec!(49000.0),
            amount: dec!(0.001),
        });
        strategy.tracked_sell = Some(TrackedOrder {
            cl_ord_id: "sell123".to_string(),
            side: Side::Sell,
            price: dec!(51000.0),
            amount: dec!(0.001),
        });

        // มี Order Update เข้ามาว่า Buy Filled
        let order_update = OkxOrderUpdateData {
            inst_id: "BTC-USDT".to_string(),
            order_id: "ord_buy_1".to_string(),
            client_order_id: "buy123".to_string(),
            tag: None,
            side: "buy".to_string(),
            px: "49000.0".to_string(),
            sz: "0.001".to_string(),
            ord_type: "post_only".to_string(),
            state: "filled".to_string(),
            fill_size: "0.001".to_string(),
            fill_price: "49000.0".to_string(),
            avg_price: "49000.0".to_string(),
            acc_fill_size: "0.001".to_string(),
            fee: "-0.0000008".to_string(),
            fee_currency: "BTC".to_string(),
            update_time: "1700000000".to_string(),
            create_time: "1700000000".to_string(),
        };

        let actions = strategy.on_order_update(&order_update);
        assert!(!actions.is_empty(), "ต้องมี action ส่งออกมา");

        // ตรวจสอบว่า Action เป็นการ Cancel ฝั่ง Sibling (sell123)
        let has_cancel_sibling = actions.iter().any(|a| {
            if let StrategyAction::CancelOrder(arg) = a {
                arg.client_order_id == Some("sell123".to_string())
            } else {
                false
            }
        });
        assert!(has_cancel_sibling, "ต้องสั่งยกเลิก Sibling (sell123)");
    }

    #[test]
    fn test_on_recovery_reconcile_replays_trades_chronologically() {
        let mut strategy = FixedRatioStrategy::new(
            "bot-replay".to_string(),
            "BTC-USDT".to_string(),
            mock_config(),
        );

        assert!(strategy.snapshot.is_none());

        // จำลอง missed trades สองรายการที่สลับลำดับเวลามา (t2 เกิดทีหลัง t1)
        let trades = vec![
            OkxMyTradeData {
                inst_id: "BTC-USDT".to_string(),
                trade_id: "t2".to_string(),
                order_id: "o2".to_string(),
                client_order_id: "c2".to_string(),
                fill_price: "51000.0".to_string(),
                fill_size: "0.001".to_string(),
                side: "sell".to_string(),
                fee: "0.04".to_string(),
                fee_currency: "USDT".to_string(),
                ts: "1700000005000".to_string(),
                exec_type: Some("M".to_string()),
            },
            OkxMyTradeData {
                inst_id: "BTC-USDT".to_string(),
                trade_id: "t1".to_string(),
                order_id: "o1".to_string(),
                client_order_id: "c1".to_string(),
                fill_price: "50000.0".to_string(),
                fill_size: "0.002".to_string(),
                side: "buy".to_string(),
                fee: "0.08".to_string(),
                fee_currency: "USDT".to_string(),
                ts: "1700000001000".to_string(),
                exec_type: Some("M".to_string()),
            },
        ];

        let res = strategy.on_recovery_reconcile(&trades);
        assert!(res.is_ok());

        // Snapshot ต้องถูก replay ทั้ง 2 รายการ (Seq 1 เป็น first rebalance และ Seq 2 เป็น second)
        let snap = strategy.snapshot.as_ref().expect("ต้องมี snapshot หลัง replay");
        assert_eq!(snap.sequence, 2);
        assert_eq!(strategy.processed_trade_ids.len(), 2);
    }
}

