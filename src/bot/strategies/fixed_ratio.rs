use rust_decimal::Decimal;
use std::str::FromStr;
use std::time::{Duration, Instant};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use crate::{
    bot::{
        math::fixed_ratio_cal::{
            RebalanceRecord, TargetAnchorOrder, cal_target_amount_improved, cal_target_price,
            generate_cl_ord_id, roundup, save_to_db_filled_rebalance,
            save_to_db_first_filled_rebalance,
        },
        strategies::base::{Strategy, StrategyAction, StrategyRecoveryFuture},
    },
    domain::{
        order::{Side, TrackedOrder},
        strategy::{StrategyState, fixed_ratio_rebalance::FixdRatioRebalanceConfig},
    },
    okx::{
        dto::{
            market::OkxTickerData,
            trade::{
                OkxMyTradeData, OkxOrderUpdateData, WsAmendOrderArg, WsCancelOrderArg,
                WsPlaceOrderArg,
            },
        },
        rest_client::{OkxClientError, OkxRestClient, OkxRestCredentials},
    },
    services::rebalance_service::RebalanceService,
};

/// สถานะการติดตามและปรับราคาของ Chasing Order
#[derive(Debug, Clone)]
pub struct ChasingState {
    pub client_order_id: String,
    pub side: Side,
    pub current_price: Decimal,
    pub anchor_price: Decimal,
    pub last_amend_time: Instant,
}

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
    rebalance_service: Option<Arc<RebalanceService>>,
    pub rest_client: Option<Arc<OkxRestClient>>,
    pub rest_credentials: Option<OkxRestCredentials>,
    pub inst_id_code: Option<u64>,

    // Runtime States
    pub snapshot: Option<RebalanceRecord>,
    pub tracked_buy: Option<TrackedOrder>,
    pub tracked_sell: Option<TrackedOrder>,
    pub parked_buy: Option<TrackedOrder>,
    pub parked_sell: Option<TrackedOrder>,
    pub chasing_state: Option<ChasingState>,

    /// ปริมาณที่ Fill สะสมแล้วต่อ clOrdId (Trade-Centric Guard)
    pub cumulative_filled: HashMap<String, Decimal>,
    /// รายการ Trade ID ที่เคยประมวลผลแล้ว (Deduplication)
    pub processed_trade_ids: HashSet<String>,
    /// ตัวดักรอ (Waiter): เมื่อ Order Closed ก่อน Trade Event มาครบ
    /// Map ord_id -> (expected_filled, cl_ord_id, side)
    pub last_trade_waiters: HashMap<String, (Decimal, String, Side)>,
    /// Order IDs already finalized, to ignore repeated terminal order updates.
    pub resolved_order_ids: HashSet<String>,
    /// รายการ clOrdId ที่เราตั้งใจสั่ง Cancel เอง (Planned Cancel) เช่น Sibling cleanup
    pub planned_cancel_order_ids: HashSet<String>,

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
            rebalance_service: None,
            rest_client: None,
            rest_credentials: None,
            inst_id_code: None,
            snapshot: None,
            tracked_buy: None,
            tracked_sell: None,
            parked_buy: None,
            parked_sell: None,
            chasing_state: None,
            cumulative_filled: HashMap::new(),
            processed_trade_ids: HashSet::new(),
            last_trade_waiters: HashMap::new(),
            resolved_order_ids: HashSet::new(),
            planned_cancel_order_ids: HashSet::new(),
            is_calculating: false,
            has_initialized: false,
        }
    }

    pub fn with_rebalance_service(mut self, rebalance_service: Arc<RebalanceService>) -> Self {
        self.rebalance_service = Some(rebalance_service);
        self
    }

    pub fn with_rest_client(
        mut self,
        rest_client: Arc<OkxRestClient>,
        credentials: Option<OkxRestCredentials>,
    ) -> Self {
        self.rest_client = Some(rest_client);
        self.rest_credentials = credentials;
        self
    }

    pub fn with_inst_id_code(mut self, inst_id_code: u64) -> Self {
        self.inst_id_code = Some(inst_id_code);
        self
    }

    /// Wrapper สำหรับดึงและ cache instIdCode ผ่าน REST API
    pub async fn fetch_and_cache_instrument_code(&mut self) -> Result<u64, OkxClientError> {
        let client = self
            .rest_client
            .as_ref()
            .ok_or_else(|| OkxClientError::ApiError {
                code: "missing_client".to_string(),
                msg: "REST client not configured".to_string(),
            })?;
        let creds = self
            .rest_credentials
            .as_ref()
            .ok_or_else(|| OkxClientError::ApiError {
                code: "missing_credentials".to_string(),
                msg: "REST credentials not configured".to_string(),
            })?;

        let code = client
            .get_instrument_code(creds, "SPOT", &self.pair)
            .await?;
        self.inst_id_code = Some(code);
        Ok(code)
    }

    /// ตรวจสอบว่าคู่เทรดตรงกับของตนเองหรือไม่
    fn is_matching_pair(&self, inst_id: &str) -> bool {
        self.pair == inst_id
    }

    fn tracks_client_order_id(&self, client_order_id: &str) -> bool {
        self.tracked_buy
            .iter()
            .chain(self.tracked_sell.iter())
            .any(|order| order.cl_ord_id == client_order_id)
            || self
                .last_trade_waiters
                .values()
                .any(|(_, waiting_client_order_id, _)| waiting_client_order_id == client_order_id)
    }

    /// Wrapper ดึง instIdCode สำหรับ OKX V5 Private WebSocket (ดึงจาก cache / preloaded)
    pub fn get_inst_id_code(&self) -> Option<u64> {
        self.inst_id_code
    }

    /// คำนวณและสร้าง Action สำหรับการ Rebalance รอบปกติ (Buy & Sell พร้อมกัน)
    fn calculate_and_place_rebalance(&mut self, base_price: Decimal) -> Vec<StrategyAction> {
        let last = match &self.snapshot {
            Some(s) => s.clone(),
            None => {
                log::warn!(
                    "[Bot #{}] ⚠️ No snapshot found for rebalance calculation",
                    self.bot_id
                );
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

    /// คำนวณราคาสำหรับ Chasing Order (อิงตรรกะ Python: Spread >= 3 ticks vs < 3 ticks + Anchor Clamp)
    pub fn calculate_chasing_price(
        &self,
        side: Side,
        bid1: Decimal,
        ask1: Decimal,
        anchor_price: Decimal,
    ) -> Decimal {
        let tick_size = self.config.decimal_place_price_qb;
        let spread_raw = ask1 - bid1;
        let spread_ticks = if !tick_size.is_zero() {
            (spread_raw / tick_size).floor()
        } else {
            Decimal::ZERO
        };

        let new_price = if spread_ticks >= Decimal::from(3) {
            // กรณี Spread กว้าง (>= 3 ticks): แทรกกลาง (Inside Spread)
            match side {
                Side::Buy => {
                    let stepped = bid1 + roundup(tick_size, tick_size);
                    stepped.min(anchor_price)
                }
                Side::Sell => {
                    let stepped = ask1 - roundup(tick_size, tick_size);
                    stepped.max(anchor_price)
                }
            }
        } else {
            // กรณี Spread แคบ (< 3 ticks): เกาะติด Best Bid/Ask พร้อมเว้นระยะ Safety 2 ticks
            let offset = roundup(tick_size * Decimal::from(2), tick_size);
            match side {
                Side::Buy => {
                    let stepped = bid1 - offset;
                    stepped.min(anchor_price)
                }
                Side::Sell => {
                    let stepped = ask1 + offset;
                    stepped.max(anchor_price)
                }
            }
        };

        new_price
    }

    /// ตรวจสอบและ Finalize Order เมื่อ Trade Events ครบถ้วน
    fn try_finalize_order(&mut self, cl_ord_id: &str, order_id: &str) -> Vec<StrategyAction> {
        let is_buy = self
            .tracked_buy
            .as_ref()
            .map_or(false, |o| o.cl_ord_id == cl_ord_id);
        let is_sell = self
            .tracked_sell
            .as_ref()
            .map_or(false, |o| o.cl_ord_id == cl_ord_id);

        if is_buy {
            self.tracked_buy = None;
        }
        if is_sell {
            self.tracked_sell = None;
        }
        if self
            .chasing_state
            .as_ref()
            .map_or(false, |c| c.client_order_id == cl_ord_id)
        {
            self.chasing_state = None;
        }

        self.cumulative_filled.remove(cl_ord_id);
        self.last_trade_waiters.remove(order_id);
        self.resolved_order_ids.insert(order_id.to_string());
        self.planned_cancel_order_ids.remove(cl_ord_id);

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

    /// สร้างคำสั่งใหม่แบบ Follow Market หลบ 100 ticks และผูก ChasingState (Self-Healing)
    /// อิงจาก Python `_create_order_intent_at_market` (L356-L403) และ Rust `create_order_at_market` (L595-L650)
    pub fn create_order_intent_at_market(
        &mut self,
        side: Side,
        amount: Decimal,
        anchor_price: Decimal,
    ) -> Vec<StrategyAction> {
        let tick_size = self.config.decimal_place_price_qb;
        let extra_offset = tick_size * Decimal::from(100);

        // วางราคาหลบ market 100 ticks จาก anchor_price
        let adjusted_price = match side {
            Side::Buy => {
                let min_price = tick_size;
                (anchor_price - extra_offset).max(min_price)
            }
            Side::Sell => anchor_price + extra_offset,
        };

        let new_cl_ord_id = generate_cl_ord_id();
        let inst_id_code = self.get_inst_id_code();

        let tracked = TrackedOrder {
            cl_ord_id: new_cl_ord_id.clone(),
            side,
            price: adjusted_price,
            amount: amount.abs(),
        };

        match side {
            Side::Buy => self.tracked_buy = Some(tracked),
            Side::Sell => self.tracked_sell = Some(tracked),
        }

        self.chasing_state = Some(ChasingState {
            client_order_id: new_cl_ord_id.clone(),
            side,
            current_price: adjusted_price,
            anchor_price,
            last_amend_time: Instant::now(),
        });

        log::info!(
            "[Bot #{}] 🩹 [Self-Healing] Re-placing order at market (100 ticks offset): Side={} Amount={} Price={} (clOrdId={}) [Anchor: {}]",
            self.bot_id,
            side.as_str(),
            amount.abs(),
            adjusted_price,
            new_cl_ord_id,
            anchor_price
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
                client_order_id: Some(new_cl_ord_id),
                tag: None,
            },
            reference_bid: None,
            reference_ask: None,
        }]
    }
}

impl Strategy for FixedRatioStrategy {
    fn runtime_state(&self) -> Option<StrategyState> {
        Some(StrategyState::FixedRatio {
            tracked_buy: self.tracked_buy.clone(),
            tracked_sell: self.tracked_sell.clone(),
        })
    }

    fn restore_runtime_state(&mut self, state: &StrategyState) {
        if let StrategyState::FixedRatio {
            tracked_buy,
            tracked_sell,
        } = state
        {
            self.tracked_buy = tracked_buy.clone();
            self.tracked_sell = tracked_sell.clone();
        }
    }

    fn pending_cancel_orders(&self) -> Vec<WsCancelOrderArg> {
        self.tracked_buy
            .iter()
            .chain(self.tracked_sell.iter())
            .map(|order| WsCancelOrderArg {
                inst_id: self.pair.clone(),
                inst_id_code: self.get_inst_id_code(),
                order_id: None,
                client_order_id: Some(order.cl_ord_id.clone()),
            })
            .collect()
    }

    fn recover<'a>(
        &'a mut self,
        missed_trades: &'a [OkxMyTradeData],
    ) -> StrategyRecoveryFuture<'a> {
        Box::pin(async move {
            let service = self.rebalance_service.clone().ok_or_else(|| {
                "RebalanceService is required for Fixed Ratio recovery".to_string()
            })?;
            let snapshot = service
                .get_last_snapshot(&self.bot_id)
                .await
                .map_err(|err| format!("Failed to load last rebalance snapshot: {err}"))?;

            if let Some(record) = &snapshot {
                log::info!(
                    "[Bot #{}] ♻️ Restored rebalance snapshot (seq={})",
                    self.bot_id,
                    record.sequence
                );
            } else {
                log::info!(
                    "[Bot #{}] No rebalance snapshot found; first-order flow is eligible",
                    self.bot_id
                );
            }

            self.snapshot = snapshot;
            self.has_initialized = false;

            let mut ordered_trades: Vec<OkxMyTradeData> = missed_trades
                .iter()
                .filter(|trade| {
                    self.is_matching_pair(&trade.inst_id)
                        && self.tracks_client_order_id(&trade.client_order_id)
                })
                .cloned()
                .collect();
            ordered_trades.sort_by(|left, right| {
                left.ts
                    .parse::<u64>()
                    .unwrap_or(0)
                    .cmp(&right.ts.parse::<u64>().unwrap_or(0))
                    .then_with(|| left.trade_id.cmp(&right.trade_id))
            });

            for trade in &ordered_trades {
                if trade.trade_id.is_empty() {
                    return Err(format!(
                        "Recovery fill for clOrdId {} has no tradeId",
                        trade.client_order_id
                    ));
                }
                if self.processed_trade_ids.contains(&trade.trade_id) {
                    continue;
                }
                if service
                    .trade_exists(&self.bot_id, &trade.trade_id)
                    .await
                    .map_err(|err| {
                        format!("Failed checking recovered trade {}: {err}", trade.trade_id)
                    })?
                {
                    self.processed_trade_ids.insert(trade.trade_id.clone());
                    continue;
                }

                let fill_price = Decimal::from_str(&trade.fill_price).map_err(|err| {
                    format!("Invalid fill price for trade {}: {err}", trade.trade_id)
                })?;
                let fill_size = Decimal::from_str(&trade.fill_size).map_err(|err| {
                    format!("Invalid fill size for trade {}: {err}", trade.trade_id)
                })?;
                if fill_price <= Decimal::ZERO || fill_size <= Decimal::ZERO {
                    return Err(format!(
                        "Invalid non-positive recovery fill {}",
                        trade.trade_id
                    ));
                }
                let side = match trade.side.to_lowercase().as_str() {
                    "buy" => Side::Buy,
                    "sell" => Side::Sell,
                    other => return Err(format!("Invalid recovery fill side '{other}'")),
                };
                let (fee_type, fee_rate) = match trade.exec_type.as_deref() {
                    Some("T") => ("taker", self.config.taker_fee),
                    _ => ("maker", self.config.maker_fee),
                };
                let signed_size = if side == Side::Sell {
                    -fill_size
                } else {
                    fill_size
                };

                let next_snapshot = match &self.snapshot {
                    Some(last) => save_to_db_filled_rebalance(
                        "limit",
                        side.as_str(),
                        &self.pair,
                        fee_type,
                        last.sequence + 1,
                        last.net_base_balance_b,
                        fill_price,
                        signed_size,
                        fee_rate,
                        last.quote_value_after_q,
                        last.base_additional_b,
                        last.quote_additional_q,
                        last.target_anchor_buy.clone(),
                        last.target_anchor_sell.clone(),
                    ),
                    None => save_to_db_first_filled_rebalance(
                        "limit",
                        side.as_str(),
                        &self.pair,
                        fee_type,
                        self.config.initial_capital_b,
                        self.config.initial_capital_q,
                        fill_price,
                        signed_size,
                        fee_rate,
                        None,
                        None,
                    ),
                };

                service
                    .save_trade_fill(
                        &self.bot_id,
                        &trade.order_id,
                        &trade.trade_id,
                        &trade.client_order_id,
                        &next_snapshot,
                    )
                    .await
                    .map_err(|err| {
                        format!(
                            "Failed persisting recovered trade {}: {err}",
                            trade.trade_id
                        )
                    })?;

                self.snapshot = Some(next_snapshot);
                self.processed_trade_ids.insert(trade.trade_id.clone());
                log::info!(
                    "[Bot #{}] 💾 Replayed recovery trade {}",
                    self.bot_id,
                    trade.trade_id
                );
            }

            // RecoveryEngine reaches this hook only after all tracked orders are terminal/absent.
            self.tracked_buy = None;
            self.tracked_sell = None;
            self.chasing_state = None;
            self.cumulative_filled.clear();
            self.last_trade_waiters.clear();
            self.is_calculating = false;

            Ok(())
        })
    }

    fn on_ticker(&mut self, ticker: &OkxTickerData) -> Vec<StrategyAction> {
        if !self.is_matching_pair(&ticker.inst_id) {
            return vec![];
        }

        // Guard: ถ้า order ที่กำลัง chase ไม่อยู่ใน tracked อีกแล้ว (ถูก Park/Finalize) ให้หยุด chase
        // ไม่งั้นจะยิง amend ไปหาคำสั่งที่ไม่มีอยู่จริงบน exchange (OKX sCode=51503)
        if let Some(chasing_id) = self
            .chasing_state
            .as_ref()
            .map(|c| c.client_order_id.clone())
        {
            let still_tracked = self
                .tracked_buy
                .as_ref()
                .is_some_and(|o| o.cl_ord_id == chasing_id)
                || self
                    .tracked_sell
                    .as_ref()
                    .is_some_and(|o| o.cl_ord_id == chasing_id);
            if !still_tracked {
                log::warn!(
                    "[Bot #{}] 🏃 [Chasing] Stopped: order {} is no longer active on this strategy (parked/finalized)",
                    self.bot_id,
                    chasing_id
                );
                self.chasing_state = None;
                return vec![];
            }
        }

        // 1. ถ้ามีคำสั่ง Chasing อยู่ในกระบวนการ ให้ประเมินและปรับราคา (Amend) เกาะตลาด
        if let Some(chasing) = &self.chasing_state {
            let bid1 = Decimal::from_str(&ticker.bid_price).unwrap_or(Decimal::ZERO);
            let ask1 = Decimal::from_str(&ticker.ask_price).unwrap_or(Decimal::ZERO);

            if bid1 > Decimal::ZERO && ask1 > Decimal::ZERO {
                let now = Instant::now();
                // Throttle: ป้องกันการยิง amend order ถี่เกิน 5000ms
                // ตั้งไว้สูงเพื่อให้ราคามีโอกาสวิ่งมาชน order เอง และลด partial fill ที่ไม่จำเป็น
                if now.duration_since(chasing.last_amend_time) >= Duration::from_millis(5000) {
                    let side = chasing.side;
                    let current_price = chasing.current_price;
                    let anchor_price = chasing.anchor_price;
                    let cl_ord_id = chasing.client_order_id.clone();

                    let new_price = self.calculate_chasing_price(side, bid1, ask1, anchor_price);

                    if new_price != current_price {
                        log::info!(
                            "[Bot #{}] 🏃 [Chasing] Amending order {}: {} -> {} (Side: {:?}, BestBid: {}, BestAsk: {}, Anchor: {})",
                            self.bot_id,
                            cl_ord_id,
                            current_price,
                            new_price,
                            side,
                            bid1,
                            ask1,
                            anchor_price
                        );

                        if let Some(chasing_mut) = &mut self.chasing_state {
                            chasing_mut.current_price = new_price;
                            chasing_mut.last_amend_time = now;
                        }

                        // อัปเดตราคาใน tracked_order ด้วยเพื่อความสอดคล้อง
                        if let Some(order) = &mut self.tracked_buy {
                            if order.cl_ord_id == cl_ord_id {
                                order.price = new_price;
                            }
                        }
                        if let Some(order) = &mut self.tracked_sell {
                            if order.cl_ord_id == cl_ord_id {
                                order.price = new_price;
                            }
                        }

                        let inst_id_code = self.get_inst_id_code();
                        return vec![StrategyAction::AmendOrder(WsAmendOrderArg {
                            inst_id: self.pair.clone(),
                            inst_id_code,
                            order_id: None,
                            client_order_id: Some(cl_ord_id),
                            new_price: Some(new_price.to_string()),
                            new_size: None,
                        })];
                    }
                }
            }
            return vec![];
        }

        // Ignore ticker while orders are tracked or after the first evaluation.
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

        // A restored snapshot starts a normal rebalance, not the initial-capital order.
        if self.snapshot.is_some() {
            return self.calculate_and_place_rebalance(current_price);
        }

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

        // กรณีที่ 2: พอร์ต Imbalanced -> วาง Order เดี่ยว 1 ฝั่งเพื่อปรับสมดุล และเริ่ม Chasing
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

        // กำหนด Anchor Price จาก snapshot target_info ถ้ามี หรือใช้ current_price เป็นเพดาน/พื้น
        let anchor_price = match side {
            Side::Buy => self
                .snapshot
                .as_ref()
                .and_then(|s| s.target_anchor_buy.as_ref())
                .map_or(current_price, |a| a.price_qb),
            Side::Sell => self
                .snapshot
                .as_ref()
                .and_then(|s| s.target_anchor_sell.as_ref())
                .map_or(current_price, |a| a.price_qb),
        };

        self.chasing_state = Some(ChasingState {
            client_order_id: cl_ord_id.clone(),
            side,
            current_price: adjusted_price,
            anchor_price,
            last_amend_time: Instant::now(),
        });

        log::info!(
            "[Bot #{}] 🚀 Placing First Single Order: Side={} Amount={} Price={} (clOrdId={}) [Chasing Anchor: {}]",
            self.bot_id,
            side.as_str(),
            amount.abs(),
            adjusted_price,
            cl_ord_id,
            anchor_price
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

        if !self.tracks_client_order_id(cl_ord_id) {
            log::debug!(
                "[Bot #{}] Ignoring order update not owned by this strategy: clOrdId={}",
                self.bot_id,
                cl_ord_id
            );
            return vec![];
        }

        if self.resolved_order_ids.contains(ord_id) && matches!(state, "filled" | "canceled") {
            log::debug!(
                "[Bot #{}] Ignoring repeated terminal update for order {}",
                self.bot_id,
                ord_id
            );
            return vec![];
        }

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
                // 1. สั่ง Cancel Sibling ทันที (ยกเลิกออเดอร์ฝั่งตรงข้าม) - Planned Cancel
                let inst_id_code = self.get_inst_id_code();
                if let Some(buy_order) = &self.tracked_buy {
                    if &buy_order.cl_ord_id != cl_ord_id {
                        log::info!(
                            "[Bot #{}] 🛑 Filled Trigger: Cancelling sibling BUY order {}",
                            self.bot_id,
                            buy_order.cl_ord_id
                        );
                        self.planned_cancel_order_ids
                            .insert(buy_order.cl_ord_id.clone());
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
                        self.planned_cancel_order_ids
                            .insert(sell_order.cl_ord_id.clone());
                        actions.push(StrategyAction::CancelOrder(WsCancelOrderArg {
                            inst_id: self.pair.clone(),
                            inst_id_code,
                            order_id: None,
                            client_order_id: Some(sell_order.cl_ord_id.clone()),
                        }));
                    }
                }

                // accFillSz คือยอดสะสมทั้ง order; fillSz คือ execution ล่าสุดเท่านั้น
                let expected_filled =
                    Decimal::from_str(&order.acc_fill_size).unwrap_or(Decimal::ZERO);
                let current_cumulative = self
                    .cumulative_filled
                    .get(cl_ord_id)
                    .cloned()
                    .unwrap_or(Decimal::ZERO);

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
                    let side = if order.side.to_lowercase() == "buy" {
                        Side::Buy
                    } else {
                        Side::Sell
                    };
                    self.last_trade_waiters
                        .insert(ord_id.clone(), (expected_filled, cl_ord_id.clone(), side));
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
                let fill_sz = Decimal::from_str(&order.acc_fill_size).unwrap_or(Decimal::ZERO);
                let current_cumulative = self
                    .cumulative_filled
                    .get(cl_ord_id)
                    .cloned()
                    .unwrap_or(Decimal::ZERO);

                if fill_sz > Decimal::ZERO && current_cumulative < fill_sz {
                    // 8.4.2 Unplanned Cancel ที่มี Partial Fill ค้าง:
                    // มี execution เกิดขึ้นจริงบน exchange แต่ trade fills ยังมาไม่ครบ
                    // 1. ลง Waiter รอให้ on_fill_update รับ trade ให้ครบแล้ว finalize
                    // 2. สั่ง Cancel sibling order ทันที เพราะ balance ไม่ตรงตามแผนเดิมแล้ว
                    let side = if order.side.to_lowercase() == "buy" {
                        Side::Buy
                    } else {
                        Side::Sell
                    };
                    self.last_trade_waiters
                        .insert(ord_id.clone(), (fill_sz, cl_ord_id.clone(), side));
                    log::info!(
                        "[Bot #{}] ⏳ [8.4.2 Canceled Partial] Waiting for trades: {} (Cumulative: {}, Expected: {})",
                        self.bot_id,
                        cl_ord_id,
                        current_cumulative,
                        fill_sz
                    );

                    // Cancel sibling order ทันที
                    let inst_id_code = self.get_inst_id_code();
                    if let Some(buy_order) = &self.tracked_buy {
                        if &buy_order.cl_ord_id != cl_ord_id {
                            log::info!(
                                "[Bot #{}] 🛑 [Partial Cancel Cleanup] Cancelling sibling BUY order {}",
                                self.bot_id,
                                buy_order.cl_ord_id
                            );
                            self.planned_cancel_order_ids
                                .insert(buy_order.cl_ord_id.clone());
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
                                "[Bot #{}] 🛑 [Partial Cancel Cleanup] Cancelling sibling SELL order {}",
                                self.bot_id,
                                sell_order.cl_ord_id
                            );
                            self.planned_cancel_order_ids
                                .insert(sell_order.cl_ord_id.clone());
                            actions.push(StrategyAction::CancelOrder(WsCancelOrderArg {
                                inst_id: self.pair.clone(),
                                inst_id_code,
                                order_id: None,
                                client_order_id: Some(sell_order.cl_ord_id.clone()),
                            }));
                        }
                    }
                } else {
                    let cancel_source = order.cancel_source.trim();
                    let s_code = order.code.trim();

                    // 8.4.3 Planned Cancel: เราสั่ง Cancel เอง (เช่น Sibling cleanup หรือ intentional cancel)
                    let is_planned = self.planned_cancel_order_ids.contains(cl_ord_id);

                    // 8.3.2 Insufficient Funds (51010/51015 / cancelSource 9, 44): ไม่ heal
                    if s_code == "51010"
                        || s_code == "51015"
                        || cancel_source == "9"
                        || cancel_source == "44"
                    {
                        log::warn!(
                            "[Bot #{}] ⚠️ Order canceled due to Insufficient Funds (code={}, cancelSource={}): clOrdId={}. Skipping healing, finalizing order.",
                            self.bot_id,
                            s_code,
                            cancel_source,
                            cl_ord_id
                        );
                        let finalize_actions = self.try_finalize_order(cl_ord_id, ord_id);
                        actions.extend(finalize_actions);
                    }
                    // Planned Cancel: ยกเลิกตามแผนของเรา (ห้าม heal)
                    else if is_planned {
                        log::info!(
                            "[Bot #{}] 🛑 [8.4.3 Planned Cancel] Order {} was intentionally cancelled. Finalizing...",
                            self.bot_id,
                            cl_ord_id
                        );
                        let finalize_actions = self.try_finalize_order(cl_ord_id, ord_id);
                        actions.extend(finalize_actions);
                    }
                    // // User Cancel จากภายนอก (cancelSource == "1" ที่ไม่ได้อยู่ใน planned_cancel_order_ids): ไม่ heal
                    // else if cancel_source == "1" {
                    //     log::info!(
                    //         "[Bot #{}] 👤 [External User Cancel] Order {} cancelled by user (cancelSource=1). Finalizing without heal.",
                    //         self.bot_id,
                    //         cl_ord_id
                    //     );
                    //     let finalize_actions = self.try_finalize_order(cl_ord_id, ord_id);
                    //     actions.extend(finalize_actions);
                    // }
                    // // 8.4.1 Unplanned Cancel (Zero Fill) & 8.3.1 Post-Only Rejection:
                    // // Trigger: cancelSource == "0" (system cancel) หรือ "31" (post-only) หรือ sCode != "0" โดยที่ fill_sz == 0
                    // else if (cancel_source == "0" || cancel_source == "31" || (s_code != "0" && !s_code.is_empty() && s_code != "50011"))
                    //     && fill_sz == Decimal::ZERO
                    // {

                    // 8.4.1 Unplanned Cancel (Zero Fill) & External Cancel:
                    // ถ้าไม่อยู่ใน planned_cancel_order_ids และไม่มี fill -> Self-Heal สวนกลับมาเสมอ (แบบ Legacy)
                    else if fill_sz == Decimal::ZERO {
                        log::warn!(
                            "[Bot #{}] 🩹 [8.4.1 Unplanned Cancel Healing] Order cancelled (cancelSource={}, sCode={}): clOrdId={}. Re-placing at market (100 ticks offset)...",
                            self.bot_id,
                            cancel_source,
                            s_code,
                            cl_ord_id
                        );

                        // 8.4.4 ถ้ากำลัง Chasing อยู่ ให้เคลียร์ chasing_state เดิมออกเพื่อกัน race condition
                        if self
                            .chasing_state
                            .as_ref()
                            .map_or(false, |c| &c.client_order_id == cl_ord_id)
                        {
                            self.chasing_state = None;
                        }

                        // ดึงข้อมูลเดิมจาก tracked order
                        let old_order = if self
                            .tracked_buy
                            .as_ref()
                            .map_or(false, |o| &o.cl_ord_id == cl_ord_id)
                        {
                            self.tracked_buy.take()
                        } else if self
                            .tracked_sell
                            .as_ref()
                            .map_or(false, |o| &o.cl_ord_id == cl_ord_id)
                        {
                            self.tracked_sell.take()
                        } else {
                            None
                        };

                        if let Some(tracked) = old_order {
                            self.cumulative_filled.remove(cl_ord_id);
                            self.last_trade_waiters.remove(ord_id);
                            self.resolved_order_ids.insert(ord_id.to_string());

                            // หา anchor_price: ถ้ามีใน snapshot ให้ใช้ anchor จาก snapshot, มิฉะนั้นใช้ tracked.price
                            let anchor_price = match tracked.side {
                                Side::Buy => self
                                    .snapshot
                                    .as_ref()
                                    .and_then(|s| s.target_anchor_buy.as_ref())
                                    .map_or(tracked.price, |a| a.price_qb),
                                Side::Sell => self
                                    .snapshot
                                    .as_ref()
                                    .and_then(|s| s.target_anchor_sell.as_ref())
                                    .map_or(tracked.price, |a| a.price_qb),
                            };

                            let heal_actions = self.create_order_intent_at_market(
                                tracked.side,
                                tracked.amount,
                                anchor_price,
                            );
                            actions.extend(heal_actions);
                        } else {
                            log::warn!(
                                "[Bot #{}] ⚠️ Tracked order {} not found during self-healing, finalizing instead.",
                                self.bot_id,
                                cl_ord_id
                            );
                            let finalize_actions = self.try_finalize_order(cl_ord_id, ord_id);
                            actions.extend(finalize_actions);
                        }
                    } else {
                        // การยกเลิกทั่วไป
                        let finalize_actions = self.try_finalize_order(cl_ord_id, ord_id);
                        actions.extend(finalize_actions);
                    }
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

        if !self.tracks_client_order_id(&fill.client_order_id) {
            log::debug!(
                "[Bot #{}] Ignoring fill not owned by this strategy: clOrdId={} tradeId={}",
                self.bot_id,
                fill.client_order_id,
                fill.trade_id
            );
            return vec![];
        }

        let trade_id = &fill.trade_id;
        let ord_id = &fill.order_id;
        let cl_ord_id = &fill.client_order_id;

        // Guard: Deduplication
        if !self.processed_trade_ids.insert(trade_id.clone()) {
            log::info!(
                "[Bot #{}] ⏭️ Skipping already processed trade: {}",
                self.bot_id,
                trade_id
            );
            return vec![];
        }

        let fill_px = Decimal::from_str(&fill.fill_price).unwrap_or(Decimal::ZERO);
        let fill_sz = Decimal::from_str(&fill.fill_size).unwrap_or(Decimal::ZERO);
        let side_str = fill.side.to_lowercase();
        let side = if side_str == "buy" {
            Side::Buy
        } else {
            Side::Sell
        };

        // 1. สะสม cumulative_filled
        let current_cum = self
            .cumulative_filled
            .entry(cl_ord_id.clone())
            .or_insert(Decimal::ZERO);
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
        let (fee_type, fee_rate) = match fill.exec_type.as_deref() {
            Some("T") => ("taker", self.config.taker_fee),
            _ => ("maker", self.config.maker_fee),
        };
        let signed_amount = if side == Side::Sell {
            -fill_sz
        } else {
            fill_sz
        };

        let new_snapshot = match &self.snapshot {
            None => {
                // First Order Fill
                save_to_db_first_filled_rebalance(
                    "limit",
                    side.as_str(),
                    &self.pair,
                    fee_type,
                    self.config.initial_capital_b,
                    self.config.initial_capital_q,
                    fill_px,
                    signed_amount,
                    fee_rate,
                    None,
                    None,
                )
            }
            Some(last) => save_to_db_filled_rebalance(
                "limit",
                side.as_str(),
                &self.pair,
                fee_type,
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
            ),
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
            .filter(|trade| {
                self.is_matching_pair(&trade.inst_id)
                    && self.tracks_client_order_id(&trade.client_order_id)
            })
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
            let side = if side_str == "buy" {
                Side::Buy
            } else {
                Side::Sell
            };
            let fee_rate = self.config.maker_fee;
            let signed_amount = if side == Side::Sell {
                -fill_sz
            } else {
                fill_sz
            };

            let new_snapshot = match &self.snapshot {
                None => save_to_db_first_filled_rebalance(
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
                ),
                Some(last) => save_to_db_filled_rebalance(
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
                ),
            };

            self.processed_trade_ids.insert(trade.trade_id.clone());
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

    fn owns_client_order_id(&self, client_order_id: &str) -> bool {
        self.tracks_client_order_id(client_order_id)
            || self
                .parked_buy
                .as_ref()
                .is_some_and(|o| o.cl_ord_id == client_order_id)
            || self
                .parked_sell
                .as_ref()
                .is_some_and(|o| o.cl_ord_id == client_order_id)
            || self
                .chasing_state
                .as_ref()
                .is_some_and(|c| c.client_order_id == client_order_id)
    }

    fn on_cem_reactivate(
        &mut self,
        reactivated_order: &crate::pipeline::simulate_cancel_plan::TrackedOrderForCem,
    ) -> Vec<StrategyAction> {
        // ตรวจสอบว่าคำสั่งนี้ตรงกับ parked_buy หรือ parked_sell หรือ tracked ของบอทตัวนี้หรือไม่
        let is_buy = self
            .parked_buy
            .as_ref()
            .map_or(false, |b| b.cl_ord_id == reactivated_order.intent_id)
            || self
                .tracked_buy
                .as_ref()
                .map_or(false, |b| b.cl_ord_id == reactivated_order.intent_id);

        let is_sell = self
            .parked_sell
            .as_ref()
            .map_or(false, |s| s.cl_ord_id == reactivated_order.intent_id)
            || self
                .tracked_sell
                .as_ref()
                .map_or(false, |s| s.cl_ord_id == reactivated_order.intent_id);

        if !is_buy && !is_sell {
            // หมายเหตุ: ไม่ใช่ capital leak — executor จะเจอ actions ว่างแล้วเรียก
            // abandon_reactivated_order คืนเงินจองกลับ free ทันที (log "Reactivation abandoned")
            log::warn!(
                "[Bot #{}] ⚠️ CEM reactivate skipped for {} (not tracked or parked in this bot) — lock will be returned to CEM free by executor",
                self.bot_id,
                reactivated_order.intent_id
            );
            return Vec::new();
        }

        let side_str = if is_buy { "buy" } else { "sell" };
        let inst_id_code = self.get_inst_id_code();

        // ย้ายจาก parked กลับมาเป็น active tracked order บนกระดานจริง
        if is_buy {
            if let Some(parked) = self.parked_buy.take() {
                self.tracked_buy = Some(parked);
            }
        } else if let Some(parked) = self.parked_sell.take() {
            self.tracked_sell = Some(parked);
        }

        // ล้าง planned flag ของ preemption เก่า: clOrdId ตัวเดิมถูกวางขึ้นกระดานใหม่แล้ว
        // -> การ cancel ครั้งถัดไป (เช่น user กด cancel บน exchange โดยตรง cancelSource=1)
        //    ต้องถูกจับเป็น unplanned แล้ว Self-Heal (8.4.1) ไม่ใช่ finalize เงียบๆ (8.4.3)
        self.planned_cancel_order_ids.remove(&reactivated_order.intent_id);

        let place_arg = WsPlaceOrderArg {
            id: None,
            inst_id: self.pair.clone(),
            inst_id_code,
            td_mode: "cash".to_string(),
            side: side_str.to_string(),
            ord_type: "post_only".to_string(),
            sz: reactivated_order.locked_amount.to_string(),
            px: Some(reactivated_order.price.to_string()),
            client_order_id: Some(reactivated_order.intent_id.clone()),
            tag: None,
        };

        log::info!(
            "[Bot #{}] 🚀 [CEM Reactivated Order] Re-placing {} order from PARKED to ACTIVE: clOrdId={}, px={}, sz={}",
            self.bot_id,
            side_str.to_uppercase(),
            reactivated_order.intent_id,
            reactivated_order.price,
            reactivated_order.locked_amount
        );

        vec![StrategyAction::PlaceOrder {
            arg: place_arg,
            reference_bid: None,
            reference_ask: None,
        }]
    }

    fn on_cem_preemption_canceled(&mut self, cl_ord_id: &str) {
        // ลงทะเบียนใน planned_cancel_order_ids เพื่อไม่ให้ on_order_update รัน healing
        self.planned_cancel_order_ids.insert(cl_ord_id.to_string());
        // หยุด chase ถ้าคำสั่งที่ถูก preempt กำลังถูก chase อยู่ (มันไม่มีบน exchange แล้ว)
        if self
            .chasing_state
            .as_ref()
            .is_some_and(|c| c.client_order_id == cl_ord_id)
        {
            self.chasing_state = None;
        }
        log::info!(
            "[Bot #{}] 🅿️ [CEM Preemption Cancel] Moving order {} to PARKED, skipping healing.",
            self.bot_id, cl_ord_id
        );
        if self.tracked_buy.as_ref().map_or(false, |b| b.cl_ord_id == cl_ord_id) {
            self.parked_buy = self.tracked_buy.take();
        }
        if self.tracked_sell.as_ref().map_or(false, |s| s.cl_ord_id == cl_ord_id) {
            self.parked_sell = self.tracked_sell.take();
        }
    }

    fn on_action_failed(&mut self, action: &StrategyAction, reason: &str) {
        if let StrategyAction::PlaceOrder { arg, .. } = action {
            if let Some(cl_ord_id) = &arg.client_order_id {
                log::warn!(
                    "[Bot #{}] 🅿️ PlaceOrder parked by CEM: clOrdId={}. Reason: {}. Moving to parked orders.",
                    self.bot_id, cl_ord_id, reason
                );
                // ย้ายจาก active tracked order บนกระดานจริงไปเป็น parked_buy/parked_sell
                if self.tracked_buy.as_ref().map_or(false, |b| &b.cl_ord_id == cl_ord_id) {
                    self.parked_buy = self.tracked_buy.take();
                }
                if self.tracked_sell.as_ref().map_or(false, |s| &s.cl_ord_id == cl_ord_id) {
                    self.parked_sell = self.tracked_sell.take();
                }
                // หยุด chase เพราะคำสั่งนี้ไม่ได้อยู่บน exchange (ถูก CEM park ก่อนวางจริง)
                if self
                    .chasing_state
                    .as_ref()
                    .is_some_and(|c| &c.client_order_id == cl_ord_id)
                {
                    self.chasing_state = None;
                }
            }
        }
    }

    /// OKX ปฏิเสธ place หลัง dispatch (เช่น 51008 insufficient balance) — order ไม่เคยขึ้น exchange
    /// และจะไม่มี terminal event มาเอง -> ต้องย้ายกลับเป็น PARKED ไม่งั้น strategy ยังถือว่า
    /// order อยู่บน exchange (เคสจริง sell ของ bot #3 หายจาก exchange โดยไม่มีใครรู้ จนกว่าจะ restart)
    fn on_place_rejected(&mut self, cl_ord_id: &str, reason: &str) {
        log::warn!(
            "[Bot #{}] ❌ [Place Rejected] clOrdId={} ({}) never reached exchange — moving back to PARKED (CEM will retry when capital frees)",
            self.bot_id, cl_ord_id, reason
        );
        if self
            .tracked_buy
            .as_ref()
            .map_or(false, |b| b.cl_ord_id == cl_ord_id)
        {
            self.parked_buy = self.tracked_buy.take();
        }
        if self
            .tracked_sell
            .as_ref()
            .map_or(false, |s| s.cl_ord_id == cl_ord_id)
        {
            self.parked_sell = self.tracked_sell.take();
        }
        if self
            .chasing_state
            .as_ref()
            .is_some_and(|c| c.client_order_id == cl_ord_id)
        {
            self.chasing_state = None;
        }
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
        let mut strategy =
            FixedRatioStrategy::new("bot-test-1".to_string(), "BTC-USDT".to_string(), config)
                .with_inst_id_code(3);

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
        assert_eq!(
            actions.len(),
            1,
            "ควรสร้าง First Single Order 1 รายการเพื่อปรับสมดุล"
        );

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
        assert!(strategy.chasing_state.is_some());
    }

    #[test]
    fn test_event_driven_fixed_ratio_first_order_balanced_seeds_and_places_both() {
        let mut config = mock_config();
        // ปรับให้ initial_capital_b * 50,000 = 5,000 USDT เท่ากับ quote_capital พอดี (Balanced)
        config.initial_capital_b = dec!(0.1);
        config.initial_capital_q = dec!(5000.0);

        let mut strategy =
            FixedRatioStrategy::new("bot-test-2".to_string(), "BTC-USDT".to_string(), config);

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
        assert_eq!(
            actions.len(),
            2,
            "พอร์ตสมดุลอยู่แล้ว ต้องสร้างคำสั่ง Buy และ Sell ทั้งคู่"
        );
        assert!(strategy.snapshot.is_some(), "ต้อง seed snapshot แรกเรียบร้อย");
        assert!(strategy.tracked_buy.is_some());
        assert!(strategy.tracked_sell.is_some());
    }

    #[test]
    fn test_runtime_state_restores_tracked_orders_for_recovery() {
        let mut original = FixedRatioStrategy::new(
            "bot-state-restore".to_string(),
            "BTC-USDT".to_string(),
            mock_config(),
        );
        original.tracked_buy = Some(TrackedOrder {
            cl_ord_id: "buy-restore".to_string(),
            side: Side::Buy,
            price: dec!(49000),
            amount: dec!(0.001),
        });
        original.tracked_sell = Some(TrackedOrder {
            cl_ord_id: "sell-restore".to_string(),
            side: Side::Sell,
            price: dec!(51000),
            amount: dec!(0.002),
        });

        let persisted_state = original.runtime_state().unwrap();
        let mut restarted = FixedRatioStrategy::new(
            "bot-state-restore".to_string(),
            "BTC-USDT".to_string(),
            mock_config(),
        )
        .with_inst_id_code(3);
        restarted.restore_runtime_state(&persisted_state);

        assert_eq!(
            restarted.tracked_buy.as_ref().unwrap().cl_ord_id,
            "buy-restore"
        );
        assert_eq!(
            restarted.tracked_sell.as_ref().unwrap().cl_ord_id,
            "sell-restore"
        );
        let cancel_orders = restarted.pending_cancel_orders();
        assert!(
            cancel_orders
                .iter()
                .all(|order| order.inst_id_code == Some(3))
        );
        assert_eq!(
            cancel_orders
                .into_iter()
                .map(|cancel| cancel.client_order_id.unwrap())
                .collect::<Vec<_>>(),
            vec!["buy-restore".to_string(), "sell-restore".to_string()]
        );
    }

    #[test]
    fn test_ticker_after_recovery_uses_snapshot_instead_of_first_order_flow() {
        let mut strategy = FixedRatioStrategy::new(
            "bot-recovered-snapshot".to_string(),
            "BTC-USDT".to_string(),
            mock_config(),
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

        strategy.on_ticker(&ticker);
        let persisted_snapshot = strategy.snapshot.clone().expect("seed snapshot");

        // Simulate a new process restoring an Idle state and the prior DB snapshot.
        strategy = FixedRatioStrategy::new(
            "bot-recovered-snapshot".to_string(),
            "BTC-USDT".to_string(),
            mock_config(),
        );
        strategy.snapshot = Some(persisted_snapshot);
        strategy.restore_runtime_state(&StrategyState::Idle);
        strategy.config.initial_capital_b = Decimal::ZERO;
        strategy.config.initial_capital_q = Decimal::ZERO;

        let actions = strategy.on_ticker(&ticker);

        assert_eq!(actions.len(), 2);
        assert!(strategy.tracked_buy.is_some());
        assert!(strategy.tracked_sell.is_some());
    }

    #[test]
    fn test_sibling_cancellation_on_filled() {
        let config = mock_config();
        let mut strategy =
            FixedRatioStrategy::new("bot-test-3".to_string(), "BTC-USDT".to_string(), config);

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
            ..Default::default()
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
    fn ignores_same_pair_events_owned_by_another_bot() {
        let mut strategy = FixedRatioStrategy::new(
            "bot-owner".to_string(),
            "BTC-USDT".to_string(),
            mock_config(),
        );
        strategy.tracked_buy = Some(TrackedOrder {
            cl_ord_id: "owner-buy".to_string(),
            side: Side::Buy,
            price: dec!(49000),
            amount: dec!(0.001),
        });

        let foreign_order = OkxOrderUpdateData {
            inst_id: "BTC-USDT".to_string(),
            order_id: "foreign-order".to_string(),
            client_order_id: "other-bot-sell".to_string(),
            side: "sell".to_string(),
            state: "filled".to_string(),
            fill_size: "0.01".to_string(),
            acc_fill_size: "0.01".to_string(),
            ..Default::default()
        };
        assert!(strategy.on_order_update(&foreign_order).is_empty());

        let foreign_fill = OkxMyTradeData {
            inst_id: "BTC-USDT".to_string(),
            trade_id: "foreign-trade".to_string(),
            order_id: "foreign-order".to_string(),
            client_order_id: "other-bot-sell".to_string(),
            fill_price: "50000".to_string(),
            fill_size: "0.01".to_string(),
            side: "sell".to_string(),
            ..Default::default()
        };
        assert!(strategy.on_fill_update(&foreign_fill).is_empty());

        assert!(strategy.snapshot.is_none());
        assert!(strategy.cumulative_filled.is_empty());
        assert_eq!(
            strategy.tracked_buy.as_ref().unwrap().cl_ord_id,
            "owner-buy"
        );
    }

    #[test]
    fn test_fill_event_updates_snapshot_once_per_trade_id() {
        let mut strategy = FixedRatioStrategy::new(
            "bot-fill-dedup".to_string(),
            "BTC-USDT".to_string(),
            mock_config(),
        );
        strategy.tracked_buy = Some(TrackedOrder {
            cl_ord_id: "client-1".to_string(),
            side: Side::Buy,
            price: dec!(50000),
            amount: dec!(0.001),
        });
        let fill = OkxMyTradeData {
            inst_id: "BTC-USDT".to_string(),
            trade_id: "trade-unique-1".to_string(),
            order_id: "order-1".to_string(),
            client_order_id: "client-1".to_string(),
            fill_price: "50000".to_string(),
            fill_size: "0.001".to_string(),
            side: "buy".to_string(),
            fee: "-0.04".to_string(),
            fee_currency: "USDT".to_string(),
            ts: "1700000000000".to_string(),
            exec_type: Some("M".to_string()),
        };

        strategy.on_fill_update(&fill);
        assert_eq!(strategy.snapshot.as_ref().unwrap().sequence, 1);
        assert_eq!(
            strategy.cumulative_filled.get("client-1"),
            Some(&dec!(0.001))
        );

        strategy.on_fill_update(&fill);
        assert_eq!(strategy.snapshot.as_ref().unwrap().sequence, 1);
        assert_eq!(
            strategy.cumulative_filled.get("client-1"),
            Some(&dec!(0.001))
        );
    }

    #[test]
    fn test_on_recovery_reconcile_replays_trades_chronologically() {
        let mut strategy = FixedRatioStrategy::new(
            "bot-replay".to_string(),
            "BTC-USDT".to_string(),
            mock_config(),
        );

        assert!(strategy.snapshot.is_none());
        strategy.tracked_buy = Some(TrackedOrder {
            cl_ord_id: "c1".to_string(),
            side: Side::Buy,
            price: dec!(50000),
            amount: dec!(0.002),
        });
        strategy.tracked_sell = Some(TrackedOrder {
            cl_ord_id: "c2".to_string(),
            side: Side::Sell,
            price: dec!(51000),
            amount: dec!(0.001),
        });

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
            OkxMyTradeData {
                inst_id: "BTC-USDT".to_string(),
                trade_id: "foreign-trade".to_string(),
                order_id: "foreign-order".to_string(),
                client_order_id: "other-bot-order".to_string(),
                fill_price: "52000.0".to_string(),
                fill_size: "0.5".to_string(),
                side: "buy".to_string(),
                fee: "0.01".to_string(),
                fee_currency: "USDT".to_string(),
                ts: "1700000002000".to_string(),
                exec_type: Some("M".to_string()),
            },
        ];

        let res = strategy.on_recovery_reconcile(&trades);
        assert!(res.is_ok());

        // Snapshot ต้องถูก replay ทั้ง 2 รายการ (Seq 1 เป็น first rebalance และ Seq 2 เป็น second)
        let snap = strategy
            .snapshot
            .as_ref()
            .expect("ต้องมี snapshot หลัง replay");
        assert_eq!(snap.sequence, 2);
        assert_eq!(strategy.processed_trade_ids.len(), 2);
    }

    #[test]
    fn test_chasing_order_loop_amends_price_and_finalizes() {
        let config = mock_config();
        let mut strategy =
            FixedRatioStrategy::new("bot-chasing".to_string(), "BTC-USDT".to_string(), config);

        // 1. Ticker ครั้งแรกทำให้เกิด Imbalance วาง Order Sell (หลบตลาด 100 ticks = 10.0)
        let ticker1 = OkxTickerData {
            inst_type: "SPOT".to_string(),
            inst_id: "BTC-USDT".to_string(),
            last: "60000.0".to_string(),
            last_size: "0.1".to_string(),
            ask_price: "60005.0".to_string(),
            ask_size: "1.0".to_string(),
            bid_price: "59995.0".to_string(),
            bid_size: "1.0".to_string(),
            high24h: "61000.0".to_string(),
            low24h: "59000.0".to_string(),
            vol24h: "100.0".to_string(),
            ts: "1700000000".to_string(),
        };

        let actions1 = strategy.on_ticker(&ticker1);
        assert_eq!(actions1.len(), 1);
        assert!(strategy.chasing_state.is_some());
        let chasing = strategy.chasing_state.as_ref().unwrap();
        let cl_ord_id = chasing.client_order_id.clone();
        assert_eq!(chasing.side, Side::Sell);
        assert_eq!(chasing.current_price, dec!(60010.0)); // 60000 + 10

        // 2. จำลองเวลาผ่านไปเพื่อผ่าน throttle 5000ms (ปรับ last_amend_time ย้อนหลัง 6 วินาที)
        strategy.chasing_state.as_mut().unwrap().last_amend_time =
            Instant::now() - Duration::from_secs(6);

        // Ticker ตัวถัดไป: spread = 60005 - 59995 = 10 (>= 3 ticks = 0.3)
        // ฝั่ง Sell จะแทรกกลาง: ask1 - 1 tick = 60005.0 - 0.1 = 60004.9
        let ticker2 = OkxTickerData {
            inst_type: "SPOT".to_string(),
            inst_id: "BTC-USDT".to_string(),
            last: "60000.0".to_string(),
            last_size: "0.1".to_string(),
            ask_price: "60005.0".to_string(),
            ask_size: "1.0".to_string(),
            bid_price: "59995.0".to_string(),
            bid_size: "1.0".to_string(),
            high24h: "61000.0".to_string(),
            low24h: "59000.0".to_string(),
            vol24h: "100.0".to_string(),
            ts: "1700000001".to_string(),
        };

        let actions2 = strategy.on_ticker(&ticker2);
        assert_eq!(actions2.len(), 1, "ควร emit StrategyAction::AmendOrder");
        if let StrategyAction::AmendOrder(arg) = &actions2[0] {
            assert_eq!(arg.client_order_id.as_deref(), Some(cl_ord_id.as_str()));
            assert_eq!(arg.new_price.as_deref(), Some("60004.9"));
        } else {
            panic!("Expected AmendOrder action");
        }

        assert_eq!(
            strategy.chasing_state.as_ref().unwrap().current_price,
            dec!(60004.9)
        );

        // 3. เมื่อ Order Filled ใน on_order_update -> chasing_state ต้องถูกเคลียร์
        let update = OkxOrderUpdateData {
            inst_id: "BTC-USDT".to_string(),
            order_id: "ord-chase-1".to_string(),
            client_order_id: cl_ord_id.clone(),
            state: "filled".to_string(),
            side: "sell".to_string(),
            px: "60004.9".to_string(),
            sz: "0.0166".to_string(),
            fill_price: "60004.9".to_string(),
            fill_size: "0.0166".to_string(),
            acc_fill_size: "0.0166".to_string(),
            fee: "-0.01".to_string(),
            fee_currency: "USDT".to_string(),
            trade_id: "trade-chase-1".to_string(),
            fill_time: "1700000002".to_string(),
            update_time: "1700000002".to_string(),
            exec_type: "M".to_string(),
            ..Default::default()
        };

        // จำลอง trade fill มาก่อนเพื่อพร้อม finalize
        strategy
            .cumulative_filled
            .insert(cl_ord_id.clone(), dec!(0.0166));
        strategy.on_order_update(&update);

        assert!(
            strategy.chasing_state.is_none(),
            "chasing_state ต้องถูกเคลียร์เมื่อ order ได้รับการ finalize"
        );
    }

    #[test]
    fn test_self_healing_post_only_rejection_replaces_at_market() {
        let config = mock_config();
        let mut strategy =
            FixedRatioStrategy::new("bot-heal".to_string(), "BTC-USDT".to_string(), config);

        // 1. Ticker ครั้งแรก วาง Sell Order (Imbalance)
        let ticker = OkxTickerData {
            inst_type: "SPOT".to_string(),
            inst_id: "BTC-USDT".to_string(),
            last: "60000.0".to_string(),
            last_size: "0.1".to_string(),
            ask_price: "60005.0".to_string(),
            ask_size: "1.0".to_string(),
            bid_price: "59995.0".to_string(),
            bid_size: "1.0".to_string(),
            high24h: "61000.0".to_string(),
            low24h: "59000.0".to_string(),
            vol24h: "100.0".to_string(),
            ts: "1700000000".to_string(),
        };

        let actions = strategy.on_ticker(&ticker);
        assert_eq!(actions.len(), 1);
        let orig_cl_ord_id = strategy
            .chasing_state
            .as_ref()
            .unwrap()
            .client_order_id
            .clone();
        let orig_amount = strategy.tracked_sell.as_ref().unwrap().amount;

        // 2. จำลอง OKX ส่ง canceled กลับมาพร้อม cancelSource="31" (Post-only will cross liquidity)
        let cancel_event = OkxOrderUpdateData {
            inst_id: "BTC-USDT".to_string(),
            order_id: "ord-rejected-1".to_string(),
            client_order_id: orig_cl_ord_id.clone(),
            state: "canceled".to_string(),
            side: "sell".to_string(),
            px: "60010.0".to_string(),
            sz: orig_amount.to_string(),
            fill_size: "0".to_string(),
            acc_fill_size: "0".to_string(),
            cancel_source: "31".to_string(),
            code: "51004".to_string(), // หรือ non-zero code
            ..Default::default()
        };

        let heal_actions = strategy.on_order_update(&cancel_event);
        assert_eq!(
            heal_actions.len(),
            1,
            "ต้อง emit PlaceOrder ใหม่เพื่อ Self-Healing"
        );

        if let StrategyAction::PlaceOrder { arg, .. } = &heal_actions[0] {
            assert_eq!(arg.side, "sell");
            assert_eq!(arg.sz, orig_amount.to_string());
            assert_eq!(arg.ord_type, "post_only");
            let new_cl_ord_id = arg.client_order_id.as_ref().expect("ต้องมี new clOrdId");
            assert_ne!(new_cl_ord_id, &orig_cl_ord_id, "clOrdId ใหม่ต้องไม่ซ้ำกับอันเดิม");

            // ตรวจสอบว่า chasing_state ถูกผูกเข้ากับคำสั่งใหม่
            let new_chasing = strategy
                .chasing_state
                .as_ref()
                .expect("ต้องมี chasing_state ใหม่");
            assert_eq!(&new_chasing.client_order_id, new_cl_ord_id);
            assert_eq!(new_chasing.side, Side::Sell);
        } else {
            panic!("Expected PlaceOrder action from self-healing");
        }
    }

    #[test]
    fn test_insufficient_funds_does_not_heal_and_finalizes() {
        let config = mock_config();
        let mut strategy =
            FixedRatioStrategy::new("bot-funds".to_string(), "BTC-USDT".to_string(), config);

        // 1. Ticker ครั้งแรก วาง Sell Order (Imbalance)
        let ticker = OkxTickerData {
            inst_type: "SPOT".to_string(),
            inst_id: "BTC-USDT".to_string(),
            last: "60000.0".to_string(),
            last_size: "0.1".to_string(),
            ask_price: "60005.0".to_string(),
            ask_size: "1.0".to_string(),
            bid_price: "59995.0".to_string(),
            bid_size: "1.0".to_string(),
            high24h: "61000.0".to_string(),
            low24h: "59000.0".to_string(),
            vol24h: "100.0".to_string(),
            ts: "1700000000".to_string(),
        };

        let actions = strategy.on_ticker(&ticker);
        assert_eq!(actions.len(), 1);
        let orig_cl_ord_id = strategy
            .chasing_state
            .as_ref()
            .unwrap()
            .client_order_id
            .clone();

        // 2. จำลอง OKX cancel ด้วยสาเหตุเงินไม่พอ (code=51010)
        let cancel_event = OkxOrderUpdateData {
            inst_id: "BTC-USDT".to_string(),
            order_id: "ord-nofunds-1".to_string(),
            client_order_id: orig_cl_ord_id.clone(),
            state: "canceled".to_string(),
            side: "sell".to_string(),
            px: "60010.0".to_string(),
            sz: "0.0166".to_string(),
            fill_size: "0".to_string(),
            acc_fill_size: "0".to_string(),
            code: "51010".to_string(),
            cancel_source: "0".to_string(),
            ..Default::default()
        };

        let _actions = strategy.on_order_update(&cancel_event);
        // ไม่มีการ heal (ไม่มี PlaceOrder action ที่สร้างคำสั่งใหม่เพื่อ heal)
        assert!(strategy.tracked_sell.is_none(), "tracked_sell ต้องถูกล้างออก");
        assert!(
            strategy.chasing_state.is_none(),
            "chasing_state ต้องถูกล้างออก"
        );
    }

    #[test]
    fn test_unplanned_cancel_zero_fill_heals_order_at_market() {
        let config = mock_config();
        let mut strategy =
            FixedRatioStrategy::new("bot-unplanned".to_string(), "BTC-USDT".to_string(), config);

        // 1. Initial ticker -> Place Sell Order (Imbalance)
        let ticker = OkxTickerData {
            inst_type: "SPOT".to_string(),
            inst_id: "BTC-USDT".to_string(),
            last: "60000.0".to_string(),
            last_size: "0.1".to_string(),
            ask_price: "60005.0".to_string(),
            ask_size: "1.0".to_string(),
            bid_price: "59995.0".to_string(),
            bid_size: "1.0".to_string(),
            high24h: "61000.0".to_string(),
            low24h: "59000.0".to_string(),
            vol24h: "100.0".to_string(),
            ts: "1700000000".to_string(),
        };

        let actions = strategy.on_ticker(&ticker);
        assert_eq!(actions.len(), 1);
        let orig_cl_ord_id = strategy
            .chasing_state
            .as_ref()
            .unwrap()
            .client_order_id
            .clone();
        let orig_amount = strategy.tracked_sell.as_ref().unwrap().amount;

        // 2. OKX system cancels order (cancelSource = "0", zero fill) -> 8.4.1 Unplanned Cancel
        let cancel_event = OkxOrderUpdateData {
            inst_id: "BTC-USDT".to_string(),
            order_id: "ord-sys-canceled".to_string(),
            client_order_id: orig_cl_ord_id.clone(),
            state: "canceled".to_string(),
            side: "sell".to_string(),
            px: "60010.0".to_string(),
            sz: orig_amount.to_string(),
            fill_size: "0".to_string(),
            acc_fill_size: "0".to_string(),
            cancel_source: "0".to_string(),
            code: "0".to_string(),
            ..Default::default()
        };

        let heal_actions = strategy.on_order_update(&cancel_event);
        assert_eq!(
            heal_actions.len(),
            1,
            "8.4.1: ต้อง emit PlaceOrder ใหม่เพื่อ Self-Heal"
        );

        if let StrategyAction::PlaceOrder { arg, .. } = &heal_actions[0] {
            assert_eq!(arg.side, "sell");
            assert_eq!(arg.sz, orig_amount.to_string());
            assert_ne!(
                arg.client_order_id.as_deref().unwrap(),
                &orig_cl_ord_id,
                "ต้องได้ clOrdId ใหม่"
            );
            // 8.4.4 Chasing state ต้องถูกย้ายไปผูกกับออเดอร์ใหม่
            let new_chasing = strategy
                .chasing_state
                .as_ref()
                .expect("ต้องมี ChasingState ใหม่");
            assert_eq!(
                &new_chasing.client_order_id,
                arg.client_order_id.as_ref().unwrap()
            );
        } else {
            panic!("Expected PlaceOrder action");
        }
    }

    #[test]
    fn test_planned_cancel_sibling_cleanup_does_not_heal() {
        let config = mock_config();
        let mut strategy =
            FixedRatioStrategy::new("bot-planned".to_string(), "BTC-USDT".to_string(), config);

        // จำลองมีทั้ง 2 ฝั่ง (Buy & Sell)
        let buy_order = TrackedOrder {
            cl_ord_id: "buy-ord-1".to_string(),
            side: Side::Buy,
            price: dec!(59000),
            amount: dec!(0.01),
        };
        let sell_order = TrackedOrder {
            cl_ord_id: "sell-ord-1".to_string(),
            side: Side::Sell,
            price: dec!(61000),
            amount: dec!(0.01),
        };
        strategy.tracked_buy = Some(buy_order.clone());
        strategy.tracked_sell = Some(sell_order.clone());

        // 1. จำลอง Buy order Filled -> Strategy ต้องสั่ง Cancel Sibling (Sell) และบันทึกลง planned_cancel_order_ids
        let fill_update = OkxOrderUpdateData {
            inst_id: "BTC-USDT".to_string(),
            order_id: "ord-buy-filled".to_string(),
            client_order_id: "buy-ord-1".to_string(),
            state: "filled".to_string(),
            side: "buy".to_string(),
            px: "59000.0".to_string(),
            sz: "0.01".to_string(),
            fill_size: "0.01".to_string(),
            acc_fill_size: "0.01".to_string(),
            ..Default::default()
        };
        strategy
            .cumulative_filled
            .insert("buy-ord-1".to_string(), dec!(0.01));

        let actions = strategy.on_order_update(&fill_update);
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], StrategyAction::CancelOrder(_)));
        assert!(strategy.planned_cancel_order_ids.contains("sell-ord-1"));

        // 2. เมื่อ Sell order ตอบกลับว่า Canceled (แม้ cancelSource จะเป็น "0" หรือ "1") -> 8.4.3 Planned Cancel ต้องไม่ heal
        let cancel_update = OkxOrderUpdateData {
            inst_id: "BTC-USDT".to_string(),
            order_id: "ord-sell-canceled".to_string(),
            client_order_id: "sell-ord-1".to_string(),
            state: "canceled".to_string(),
            side: "sell".to_string(),
            px: "61000.0".to_string(),
            sz: "0.01".to_string(),
            fill_size: "0".to_string(),
            acc_fill_size: "0".to_string(),
            cancel_source: "0".to_string(), // แม้ระบบ exchange บอก source=0 แต่เราจำได้ว่าเป็น planned
            ..Default::default()
        };

        let cancel_actions = strategy.on_order_update(&cancel_update);
        // ตรวจสอบว่าไม่มีการ heal (ไม่มีคำสั่ง PlaceOrder เกิดขึ้น)
        let has_place_order = cancel_actions
            .iter()
            .any(|a| matches!(a, StrategyAction::PlaceOrder { .. }));
        assert!(
            !has_place_order,
            "8.4.3: Planned cancel ห้ามวาง order ใหม่ (ห้าม heal)"
        );
        assert!(strategy.tracked_sell.is_none(), "tracked_sell ต้องถูกล้างออก");
        assert!(
            !strategy.planned_cancel_order_ids.contains("sell-ord-1"),
            "ต้องล้าง planned set ออก"
        );
    }

    #[test]
    fn test_unplanned_cancel_with_partial_fill_waits_and_cancels_sibling() {
        let config = mock_config();
        let mut strategy =
            FixedRatioStrategy::new("bot-partial".to_string(), "BTC-USDT".to_string(), config);

        let buy_order = TrackedOrder {
            cl_ord_id: "buy-ord-2".to_string(),
            side: Side::Buy,
            price: dec!(59000),
            amount: dec!(0.02),
        };
        let sell_order = TrackedOrder {
            cl_ord_id: "sell-ord-2".to_string(),
            side: Side::Sell,
            price: dec!(61000),
            amount: dec!(0.02),
        };
        strategy.tracked_buy = Some(buy_order);
        strategy.tracked_sell = Some(sell_order);

        // 8.4.2 Order Canceled แต่มี Partial Fill เกิดขึ้นแล้ว 0.01 (acc_fill_sz = 0.01 > 0)
        let cancel_partial_event = OkxOrderUpdateData {
            inst_id: "BTC-USDT".to_string(),
            order_id: "ord-buy-partial".to_string(),
            client_order_id: "buy-ord-2".to_string(),
            state: "canceled".to_string(),
            side: "buy".to_string(),
            px: "59000.0".to_string(),
            sz: "0.02".to_string(),
            fill_size: "0.01".to_string(),
            acc_fill_size: "0.01".to_string(),
            cancel_source: "0".to_string(),
            ..Default::default()
        };

        let actions = strategy.on_order_update(&cancel_partial_event);
        // ต้องสั่ง Cancel Sibling (Sell) ทันทีเพื่อความปลอดภัยของ balance
        assert_eq!(actions.len(), 1, "ต้องสั่ง Cancel Sibling SELL order");
        if let StrategyAction::CancelOrder(arg) = &actions[0] {
            assert_eq!(arg.client_order_id.as_deref(), Some("sell-ord-2"));
        } else {
            panic!("Expected CancelOrder action for sibling");
        }

        // ต้องลง waiter ไว้เพื่อรอ trade event
        assert!(strategy.last_trade_waiters.contains_key("ord-buy-partial"));
    }

    #[test]
    fn test_external_user_cancel_source_1_triggers_self_heal() {
        let config = mock_config();
        let mut strategy = FixedRatioStrategy::new(
            "bot-user-cancel".to_string(),
            "BTC-USDT".to_string(),
            config,
        );

        let buy_order = TrackedOrder {
            cl_ord_id: "buy-external-1".to_string(),
            side: Side::Buy,
            price: dec!(59000),
            amount: dec!(0.01),
        };
        strategy.tracked_buy = Some(buy_order);

        // ผู้ใช้กดยกเลิกเองบน Web / App ของ OKX (cancelSource = "1") แต่ไม่อยู่ใน planned_cancel_order_ids
        let cancel_event = OkxOrderUpdateData {
            inst_id: "BTC-USDT".to_string(),
            order_id: "ord-ext-1".to_string(),
            client_order_id: "buy-external-1".to_string(),
            state: "canceled".to_string(),
            side: "buy".to_string(),
            px: "59000.0".to_string(),
            sz: "0.01".to_string(),
            fill_size: "0".to_string(),
            acc_fill_size: "0".to_string(),
            cancel_source: "1".to_string(),
            ..Default::default()
        };

        let actions = strategy.on_order_update(&cancel_event);
        // แบบ Legacy: ต้อง Self-Heal สวนกลับมา
        let has_place_order = actions
            .iter()
            .any(|a| matches!(a, StrategyAction::PlaceOrder { .. }));
        assert!(
            has_place_order,
            "User cancel (cancelSource=1) ต้อง Self-Heal สวนกลับมาแบบ Legacy"
        );
        assert!(
            strategy.tracked_buy.is_some(),
            "tracked_buy ต้องมีตัวใหม่ที่ถูก heal"
        );
        assert!(
            strategy.chasing_state.is_some(),
            "ต้องตั้ง chasing state สำหรับ order ที่ heal"
        );
    }

    #[test]
    fn parking_order_stops_chasing_and_ownership_covers_parked_orders() {
        let mut strategy = FixedRatioStrategy::new(
            "bot-test-park".to_string(),
            "BTC-USDT".to_string(),
            mock_config(),
        );

        let cl_ord_id = "cl_sell_park_1".to_string();
        strategy.tracked_sell = Some(TrackedOrder {
            cl_ord_id: cl_ord_id.clone(),
            side: Side::Sell,
            price: dec!(50000.0),
            amount: dec!(0.001),
        });
        strategy.chasing_state = Some(ChasingState {
            client_order_id: cl_ord_id.clone(),
            side: Side::Sell,
            current_price: dec!(50000.0),
            anchor_price: dec!(49990.0),
            last_amend_time: Instant::now(),
        });
        assert!(strategy.owns_client_order_id(&cl_ord_id));
        assert!(!strategy.owns_client_order_id("cl_other_bot"));

        let action = StrategyAction::PlaceOrder {
            arg: WsPlaceOrderArg {
                id: None,
                inst_id: "BTC-USDT".to_string(),
                inst_id_code: None,
                td_mode: "cash".to_string(),
                side: "sell".to_string(),
                ord_type: "post_only".to_string(),
                sz: "0.001".to_string(),
                px: Some("50000.0".to_string()),
                client_order_id: Some(cl_ord_id.clone()),
                tag: None,
            },
            reference_bid: None,
            reference_ask: None,
        };
        strategy.on_action_failed(&action, "CEM Gatekeeper park");

        assert!(strategy.tracked_sell.is_none());
        assert!(strategy.parked_sell.is_some());
        assert!(
            strategy.chasing_state.is_none(),
            "ต้องหยุด chase เมื่อ order ถูก park (ไม่งั้นจะ amend order ที่ไม่มีบน exchange -> 51503)"
        );
        assert!(
            strategy.owns_client_order_id(&cl_ord_id),
            "เจ้าของต้องยังครอบครอง order ที่ถูก park อยู่"
        );
    }

    /// Regression จาก zzz.log 2026-10-13 step 4: sell ที่เคยถูก CEM preempt แล้วถูก reactivate
    /// ด้วย clOrdId เดิม -> เมื่อ user กด cancel บน exchange โดยตรง (cancelSource=1)
    /// ต้องถูกจับเป็น Unplanned Cancel แล้ว Self-Heal (วางใหม่) ไม่ใช่ finalize เฉยๆ
    /// (เดิม planned_cancel_order_ids ที่ insert ตอน preempt ค้างอยู่หลัง re-place -> 8.4.3 กินซะก่อน)
    #[test]
    fn test_reactivated_order_external_cancel_self_heals() {
        let config = mock_config();
        let mut strategy = FixedRatioStrategy::new(
            "bot-reactivate-cancel".to_string(),
            "BTC-USDT".to_string(),
            config,
        );

        strategy.tracked_sell = Some(TrackedOrder {
            cl_ord_id: "sell-react-1".to_string(),
            side: Side::Sell,
            price: dec!(61000),
            amount: dec!(0.01),
        });

        // 1. CEM preempt: ยกเลิก sell -> insert เข้า planned_cancel_order_ids + ย้ายไป parked
        strategy.on_cem_preemption_canceled("sell-react-1");
        assert!(strategy.tracked_sell.is_none());
        assert!(strategy.parked_sell.is_some());
        assert!(
            strategy.planned_cancel_order_ids.contains("sell-react-1"),
            "state ก่อน reactivate ต้องมี planned flag (precondition ของบั๊ก)"
        );

        // 2. เงินว่าง -> CEM reactivate: re-place ด้วย clOrdId เดิม ต้องล้าง planned flag เก่าด้วย
        let reactivated = crate::pipeline::simulate_cancel_plan::TrackedOrderForCem {
            intent_id: "sell-react-1".to_string(),
            symbol: "BTC-USDT".to_string(),
            required_asset: "BTC".to_string(),
            price: dec!(61000),
            locked_amount: dec!(0.01),
            can_be_canceled: true,
            cem_priority: Some(dec!(0.1)),
            status: crate::pipeline::simulate_cancel_plan::IntentStatus::Approved,
            bot_id: "bot-reactivate-cancel".to_string(),
            created_at: Some(1),
            inst_id_code: None,
            live_confirmed: false,
        };
        let reactivate_actions = strategy.on_cem_reactivate(&reactivated);
        assert_eq!(reactivate_actions.len(), 1, "ต้องสั่ง re-place 1 คำสั่ง");
        assert!(strategy.tracked_sell.is_some(), "tracked_sell ต้องกลับมา");
        assert!(
            !strategy.planned_cancel_order_ids.contains("sell-react-1"),
            "flag ของ preemption เก่าต้องถูกล้างตอน reactivate"
        );

        // 3. User กด cancel บน exchange โดยตรง (cancelSource=1, fill=0) -> ต้อง Self-Heal
        let cancel_event = OkxOrderUpdateData {
            inst_id: "BTC-USDT".to_string(),
            order_id: "ord-react-canceled".to_string(),
            client_order_id: "sell-react-1".to_string(),
            state: "canceled".to_string(),
            side: "sell".to_string(),
            px: "61000.0".to_string(),
            sz: "0.01".to_string(),
            fill_size: "0".to_string(),
            acc_fill_size: "0".to_string(),
            cancel_source: "1".to_string(),
            ..Default::default()
        };
        let actions = strategy.on_order_update(&cancel_event);
        let healed = actions
            .iter()
            .find_map(|a| match a {
                StrategyAction::PlaceOrder { arg, .. } => Some(arg),
                _ => None,
            })
            .expect("cancel ภายนอกหลัง reactivate ต้อง Self-Heal (วางใหม่) ไม่ใช่ finalize");
        assert_eq!(healed.side, "sell");
        assert_eq!(healed.sz, "0.01");
        assert_ne!(
            healed.client_order_id.as_deref(),
            Some("sell-react-1"),
            "ต้องได้ clOrdId ใหม่"
        );
        assert!(strategy.tracked_sell.is_some());
        assert!(strategy.chasing_state.is_some());
    }

    /// Regression ของ incident app_2026-10-13_03-55 (สมมติฐาน H3):
    /// OKX ปฏิเสธ place หลัง dispatch (51008 insufficient balance) -> ไม่มี terminal event มาเอง
    /// strategy ต้องย้าย tracked กลับเป็น PARKED (รอ CEM ปลุกใหม่) + หยุด chase
    /// ไม่งั้น bot ยังคิดว่า order อยู่บน exchange -> chase ใส่ order ที่ไม่มีจริง (51503)
    /// และไม่มีใคร rebalance ฝั่งนี้จนกว่าจะ restart
    #[test]
    fn place_rejected_moves_tracked_sell_back_to_parked_and_stops_chasing() {
        let mut strategy = FixedRatioStrategy::new(
            "bot-place-reject".to_string(),
            "BTC-USDT".to_string(),
            mock_config(),
        );
        let cl_ord_id = "cl_place_rejected".to_string();
        strategy.tracked_sell = Some(TrackedOrder {
            cl_ord_id: cl_ord_id.clone(),
            side: Side::Sell,
            price: dec!(50000.0),
            amount: dec!(0.001),
        });
        strategy.chasing_state = Some(ChasingState {
            client_order_id: cl_ord_id.clone(),
            side: Side::Sell,
            current_price: dec!(50000.0),
            anchor_price: dec!(49990.0),
            last_amend_time: Instant::now(),
        });

        strategy.on_place_rejected(
            &cl_ord_id,
            "51008 Order failed. Insufficient SUI balance in account.",
        );

        assert!(strategy.tracked_sell.is_none());
        assert!(
            strategy.parked_sell.is_some(),
            "ต้องย้ายกลับเป็น PARKED เพื่อรอ CEM ปลุกใหม่เมื่อเงินพอ"
        );
        assert!(
            strategy.chasing_state.is_none(),
            "ต้องหยุด chase เพราะ order ไม่ได้อยู่บน exchange"
        );
        assert!(
            strategy.owns_client_order_id(&cl_ord_id),
            "เจ้าของต้องยังครอบครอง order เพื่อรับ reactivation จาก CEM"
        );

        // คำสั่งของบอทอื่นห้ามโดนแตะ
        strategy.on_place_rejected("cl_other_bot", "51008");
        assert!(strategy.parked_sell.is_some());
        assert!(strategy.tracked_sell.is_none());
    }
}
