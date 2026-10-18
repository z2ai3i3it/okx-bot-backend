use std::sync::Arc;
use std::str::FromStr;
use std::time::Duration;
use rust_decimal::Decimal;
use tokio_util::sync::CancellationToken;

use crate::{
    bot::{
        recovery::{RecoveryEngine, RecoveryError},
        strategies::{base::Strategy, fixed_ratio::FixedRatioStrategy},
    },
    domain::strategy::{Strategy as StrategyModel, StrategyStatus},
    okx::{
        dto::{
            market::OkxTickerData,
            trade::{OkxMyTradeData, OkxOrderUpdateData},
        },
        ws_client::OkxPublicWsClient,
        ws_trade::OkxWsTradeClient,
    },
    pipeline::order_pipeline::OrderPipeline,
    services::rebalance_service::RebalanceService,
    storage::repositories::strategy_repository::StrategyRepository,
    okx::rest_client::{OkxRestClient, OkxRestCredentials},
};

/// Executor สำหรับรัน Event Loop ของบอท 1 ตัวใน Background Task
pub struct BotExecutor {
    strategy_model: StrategyModel,
    strategy_engine: Box<dyn Strategy>,
    public_ws: OkxPublicWsClient,
    trade_client: OkxWsTradeClient,
    pipeline: OrderPipeline,
    rebalance_service: Option<Arc<RebalanceService>>,
    strategy_repository: Option<Arc<StrategyRepository>>,
    rest_client: Option<Arc<OkxRestClient>>,
    rest_credentials: Option<OkxRestCredentials>,
    recovery_trades: Vec<OkxMyTradeData>,
    cancel_token: CancellationToken,
}

impl BotExecutor {
    pub fn new(
        strategy_model: StrategyModel,
        public_ws: OkxPublicWsClient,
        trade_client: OkxWsTradeClient,
        pipeline: OrderPipeline,
        cancel_token: CancellationToken,
    ) -> Self {
        Self::with_dependencies(
            strategy_model,
            public_ws,
            trade_client,
            pipeline,
            None,
            None,
            None,
            None,
            Vec::new(),
            cancel_token,
            None,
        )
    }

    pub fn with_dependencies(
        strategy_model: StrategyModel,
        public_ws: OkxPublicWsClient,
        trade_client: OkxWsTradeClient,
        pipeline: OrderPipeline,
        rebalance_service: Option<Arc<RebalanceService>>,
        strategy_repository: Option<Arc<StrategyRepository>>,
        rest_client: Option<Arc<OkxRestClient>>,
        rest_credentials: Option<OkxRestCredentials>,
        recovery_trades: Vec<OkxMyTradeData>,
        cancel_token: CancellationToken,
        inst_id_code: Option<u64>,
    ) -> Self {
        let fixed_ratio_rebalance_service = rebalance_service.clone();
        let mut strategy_engine: Box<dyn Strategy> = match &strategy_model.config {
            crate::domain::strategy::StrategyConfig::FixedRatioRebalance(cfg) => {
                let mut strategy = FixedRatioStrategy::new(
                    strategy_model.id.clone(),
                    strategy_model.pair.clone(),
                    cfg.clone(),
                );
                if let Some(client) = &rest_client {
                    strategy = strategy.with_rest_client(client.clone(), rest_credentials.clone());
                }
                if let Some(code) = inst_id_code {
                    strategy = strategy.with_inst_id_code(code);
                }
                if let Some(service) = fixed_ratio_rebalance_service {
                    strategy = strategy.with_rebalance_service(service);
                }
                Box::new(strategy)
            }
            crate::domain::strategy::StrategyConfig::Grid(_) => {
                // สำหรับ Grid ให้ใช้ mock grid strategy หรือ fallback
                let mut strategy = crate::bot::strategies::grid::GridStrategy::new(
                    strategy_model.id.clone(),
                    strategy_model.pair.clone(),
                    "0.0001".to_string(),
                );
                if let Some(code) = inst_id_code {
                    strategy = strategy.with_inst_id_code(code);
                }
                Box::new(strategy)
            }
        };

        strategy_engine.restore_runtime_state(&strategy_model.state);

        Self {
            strategy_model,
            strategy_engine,
            public_ws,
            trade_client,
            pipeline,
            rebalance_service,
            strategy_repository,
            rest_client,
            rest_credentials,
            recovery_trades,
            cancel_token,
        }
    }

    fn fill_from_order_update(order: &OkxOrderUpdateData) -> Option<OkxMyTradeData> {
        if order.trade_id.trim().is_empty() {
            return None;
        }

        let fill_size = Decimal::from_str(&order.fill_size).ok()?;
        if fill_size <= Decimal::ZERO {
            return None;
        }

        Some(OkxMyTradeData {
            inst_id: order.inst_id.clone(),
            trade_id: order.trade_id.clone(),
            order_id: order.order_id.clone(),
            client_order_id: order.client_order_id.clone(),
            fill_price: order.fill_price.clone(),
            fill_size: order.fill_size.clone(),
            side: order.side.clone(),
            fee: order.fill_fee.clone(),
            fee_currency: order.fill_fee_currency.clone(),
            ts: if order.fill_time.is_empty() {
                order.update_time.clone()
            } else {
                order.fill_time.clone()
            },
            exec_type: (!order.exec_type.is_empty()).then(|| order.exec_type.clone()),
        })
    }

    /// คำนวณ Mid Price จาก ticker (ใช้ค่าเฉลี่ย bid/ask ถ้ามี ไม่งั้น fallback เป็น last)
    fn mid_price_from_ticker(ticker: &OkxTickerData) -> Option<Decimal> {
        let bid = Decimal::from_str(&ticker.bid_price)
            .ok()
            .filter(|v| *v > Decimal::ZERO);
        let ask = Decimal::from_str(&ticker.ask_price)
            .ok()
            .filter(|v| *v > Decimal::ZERO);
        if let (Some(bid), Some(ask)) = (bid, ask) {
            return Some((bid + ask) / Decimal::TWO);
        }
        Decimal::from_str(&ticker.last)
            .ok()
            .filter(|v| *v > Decimal::ZERO)
    }

    /// ปลุก Parked Orders (global เรียงตาม Priority) แล้ว route กลับไปให้ "บอทเจ้าของ" วางใหม่
    /// เรียกได้จากทุก event ที่ทำให้เงินว่างขึ้น (terminal order / balance update / ticker)
    ///
    /// - reevaluate ต้อง global: ผู้แพ้ preemption ก็เป็นคนปลดล็อกเงินด้วย ถ้า owner-scoped
    ///   มันจะปลุกตัวเองก่อนแล้วแย่งเงินคืนจากผู้ชนะ (priority สูงกว่า)
    /// - route ต้องกลับเจ้าของ: ทุก executor เห็น event เดียวกัน ถ้า executor ผิดตัวเป็นคน
    ///   consume จะเรียก on_cem_reactivate กับ strategy ที่ไม่ได้เป็นเจ้าของ -> ไม่มีใครวาง order จริง
    async fn reevaluate_and_route(&self) {
        let Some(cem) = self.pipeline.cem_manager() else {
            return;
        };
        let reactivated = cem.reevaluate_parked_orders().await;
        for order in reactivated {
            cem.route_reactivation(order).await;
        }
    }

    async fn persist_runtime_state(&self, bot_id: &str) -> bool {
        let (Some(repository), Some(state)) = (
            &self.strategy_repository,
            self.strategy_engine.runtime_state(),
        ) else {
            return true;
        };

        match repository.update_runtime_state(bot_id, state).await {
            Ok(true) => true,
            Ok(false) => {
                log::error!(
                    "[Bot #{}] ❌ Strategy runtime state was not persisted: strategy not found",
                    bot_id
                );
                false
            }
            Err(err) => {
                log::error!(
                    "[Bot #{}] ❌ Failed to persist strategy runtime state: {}",
                    bot_id,
                    err
                );
                false
            }
        }
    }

    async fn process_strategy_actions(
        &mut self,
        bot_id: &str,
        account_id: &str,
        pair: &str,
        actions: Vec<crate::bot::strategies::base::StrategyAction>,
    ) {
        if actions.is_empty() {
            let _ = self.persist_runtime_state(bot_id).await;
            return;
        }

        let persisted = self.persist_runtime_state(bot_id).await;
        let actions = if persisted {
            actions
        } else {
            log::error!(
                "[Bot #{}] Blocking place/amend actions because tracked state was not persisted",
                bot_id
            );
            actions
                .into_iter()
                .filter(|action| {
                    matches!(action, crate::bot::strategies::base::StrategyAction::CancelOrder(_))
                })
                .collect()
        };

        if actions.is_empty() {
            return;
        }

        let results = self
            .pipeline
            .process_actions(bot_id, account_id, pair, actions.clone())
            .await;
        for (i, result) in results.into_iter().enumerate() {
            if let Err(err) = result {
                if err.starts_with("CEM Gatekeeper requires canceling") {
                    // Flow ปกติของ preemption: CEM สั่งยกเลิก order ไกล mid ก่อนเพื่อแย่งเงิน
                    // -> ไม่ใช่ error — strategy จะ retry order นี้เองเมื่อเงินว่าง
                    log::info!(
                        "[Bot #{}] ℹ️ Pipeline deferred by CEM preemption (will retry): {}",
                        bot_id, err
                    );
                } else {
                    log::error!("[Bot #{}] ❌ Strategy pipeline action failed: {}", bot_id, err);
                }
                if let Some(act) = actions.get(i) {
                    self.strategy_engine.on_action_failed(act, &err);
                }
            }
        }
    }

    async fn process_fill_update(
        &mut self,
        fill: &OkxMyTradeData,
        bot_id: &str,
        account_id: &str,
        pair: &str,
    ) {
        let previous_sequence = self
            .strategy_engine
            .get_latest_snapshot()
            .map(|snapshot| snapshot.sequence);
        let actions = self.strategy_engine.on_fill_update(fill);
        let snapshot = self.strategy_engine.get_latest_snapshot();
        let is_new_fill = snapshot.as_ref().is_some_and(|snapshot| {
            previous_sequence.map_or(true, |sequence| snapshot.sequence > sequence)
        });

        // Snapshot sequence advances only when the strategy accepts a unique fill.
        // Await each insert to preserve the same order as the exchange events.
        if is_new_fill {
            if let (Some(rebalance_service), Some(snapshot)) =
                (&self.rebalance_service, snapshot.as_ref())
            {
                if let Err(err) = rebalance_service
                    .save_trade_fill(
                        bot_id,
                        &fill.order_id,
                        &fill.trade_id,
                        &fill.client_order_id,
                        snapshot,
                    )
                    .await
                {
                    log::error!(
                        "[Bot #{}] ❌ Failed to persist fill {} to rebalances: {}",
                        bot_id,
                        fill.trade_id,
                        err
                    );
                } else {
                    log::info!(
                        "[Bot #{}] 💾 Fill {} persisted to rebalances (seq={})",
                        bot_id,
                        fill.trade_id,
                        snapshot.sequence
                    );
                }
            } else if snapshot.is_some() {
                log::error!(
                    "[Bot #{}] ❌ RebalanceService is not configured; fill {} was not persisted",
                    bot_id,
                    fill.trade_id
                );
            }
        }

        self.process_strategy_actions(bot_id, account_id, pair, actions)
            .await;
    }

    /// รัน Loop การทำงานของบอท (Async Event Loop)
    pub async fn run(mut self) {
        let bot_id = self.strategy_model.id.clone();
        let bot_name = self.strategy_model.name.clone();
        let account_id = self.strategy_model.account_id.clone();
        let pair = self.strategy_model.pair.clone();

        log::info!(
            "[Bot #{}] 🚀 Starting executor for bot '{}' (Pair: {})",
            bot_id,
            bot_name,
            pair
        );

        // 1. ดึง Receiver สำหรับฟังราคาตลาดสด (Public WS)
        let mut ticker_rx = self.public_ws.subscribe_ticker();

        // 2. ดึง Receiver สำหรับฟัง Private Streams
        let mut orders_rx = self.trade_client.subscribe_orders();
        let mut trades_rx = self.trade_client.subscribe_my_trades();
        let mut balance_rx = self.trade_client.subscribe_balance();
        let mut op_ack_rx = self.trade_client.subscribe_op_acks();

        // Subscribe before recovery so exchange updates are buffered during DB reads/cancels.
        loop {
            let pending_orders = self.strategy_engine.pending_cancel_orders();
            match RecoveryEngine::run_recovery(
                &bot_id,
                &pair,
                &pending_orders,
                &self.trade_client,
                self.rest_client.as_deref(),
                self.rest_credentials.as_ref(),
                &self.recovery_trades,
                self.strategy_engine.as_mut(),
            )
            .await
            {
                Ok(()) => {
                    self.persist_runtime_state(&bot_id).await;
                    break;
                }
                Err(RecoveryError::StrategyError(err)) => {
                    log::error!(
                        "[Bot #{}] 🛑 Recovery is blocked by strategy state: {}",
                        bot_id,
                        err
                    );
                    if let Some(repository) = &self.strategy_repository {
                        if let Err(status_err) = repository
                            .update_status(
                                &bot_id,
                                StrategyStatus::Error(format!("Recovery blocked: {err}")),
                            )
                            .await
                        {
                            log::error!(
                                "[Bot #{}] ❌ Failed to persist recovery error status: {}",
                                bot_id,
                                status_err
                            );
                        }
                    }
                    return;
                }
                Err(err) => {
                    log::error!(
                        "[Bot #{}] ❌ Recovery failed; delaying live strategy processing: {}",
                        bot_id,
                        err
                    );
                    tokio::select! {
                        _ = self.cancel_token.cancelled() => {
                            log::info!("[Bot #{}] Stop requested during recovery", bot_id);
                            return;
                        }
                        _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                    }
                }
            }
        }

        // 3. ดึง Initial Balance ผ่าน REST API เพื่ออุ่นเครื่อง CEM ให้พร้อมทันทีก่อนรับ Ticker แรก
        if let (Some(cem), Some(rest_client), Some(credentials)) = (
            self.pipeline.cem_manager(),
            &self.rest_client,
            &self.rest_credentials,
        ) {
            log::info!("[Bot #{}] 💰 Fetching initial account balance via REST to initialize CEM...", bot_id);
            match rest_client
                .get_balance(
                    &credentials.api_key,
                    &credentials.secret_key,
                    &credentials.passphrase,
                    credentials.is_simulated,
                )
                .await
            {
                Ok(balance_data) => {
                    for detail in balance_data.details {
                        let avail_bal = Decimal::from_str(&detail.available_balance).unwrap_or(Decimal::ZERO);
                        let total_eq = Decimal::from_str(&detail.equity).unwrap_or(avail_bal);
                        cem.update_balance_from_ws(&detail.ccy, avail_bal, total_eq).await;
                    }
                    log::info!("[Bot #{}] ✅ Initial CEM balances successfully initialized via REST", bot_id);
                }
                Err(err) => {
                    log::warn!(
                        "[Bot #{}] ⚠️ Could not fetch initial balance via REST: {}. Relying on WebSocket balance stream.",
                        bot_id, err
                    );
                }
            }
        }

        // 3. Route สำหรับรับ Reactivated Orders ของบอทนี้ (CEM ส่งกลับมาให้เจ้าของวางใหม่)
        let (reactivation_tx, mut reactivation_rx) = tokio::sync::mpsc::unbounded_channel();
        let _reactivation_tx = reactivation_tx.clone(); // ให้ channel เปิดอยู่ตลอด run()
        if let Some(cem) = self.pipeline.cem_manager() {
            cem.set_trade_client(self.trade_client.clone()).await;
            cem.register_reactivation_route(bot_id.clone(), reactivation_tx)
                .await;
        }

        // 4. Heartbeat timer เพื่อตรวจเช็คสถานะ
        let mut heartbeat_timer = tokio::time::interval(Duration::from_secs(10));

        loop {
            tokio::select! {
                // 1. ตรวจสอบสัญญาณ Cancel (เมื่อผู้ใช้กด Stop บอท)
                _ = self.cancel_token.cancelled() => {
                    log::info!(
                        "[Bot #{}] 🛑 Cancellation requested. Gracefully stopping executor for '{}'...",
                        bot_id,
                        bot_name
                    );
                    break;
                }

                // 2. รับข้อมูลราคาตลาดสดจาก Public WebSocket
                ticker_result = ticker_rx.recv() => {
                    match ticker_result {
                        Ok(ticker) => {
                            // อัปเดต Mid Price ของ "ทุกคู่" เข้า CEM (รวม BTC-USDT ที่เป็นอ้างอิงของ Cancel Plan)
                            if let Some(cem) = self.pipeline.cem_manager() {
                                if let Some(mid_px) = Self::mid_price_from_ticker(&ticker) {
                                    cem.update_mid_price(&ticker.inst_id, mid_px).await;
                                }
                            }

                            if ticker.inst_id == pair {
                                // เงินอาจว่างขึ้นจากบอทอื่น (cancel/fill) -> ปลุกตาม priority แล้ว route กลับเจ้าของ
                                self.reevaluate_and_route().await;

                                // ส่ง Ticker เข้า Strategy เพื่อประเมิน
                                let actions = self.strategy_engine.on_ticker(&ticker);

                                if !actions.is_empty() {
                                    log::info!(
                                        "[Bot #{}] ⚡ Strategy emitted {} action(s) from ticker update",
                                        bot_id,
                                        actions.len()
                                    );
                                    self.process_strategy_actions(
                                        &bot_id,
                                        &account_id,
                                        &pair,
                                        actions
                                    ).await;
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            log::warn!(
                                "[Bot #{}] ⚠️ Ticker stream lagged by {} messages",
                                bot_id,
                                skipped
                            );
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            log::error!("[Bot #{}] ❌ Ticker broadcast channel closed", bot_id);
                            break;
                        }
                    }
                }

                // 3. รับอัปเดตสถานะ Orders (Private WS)
                order_result = orders_rx.recv() => {
                    match order_result {                        Ok(order) => {
                            // Private WS ของ account เดียวกันส่ง order ของ "ทุกบอท" มาให้ทุก connection
                            // เฉพาะ executor ของบอทเจ้าของเท่านั้นที่แตะ preemption flag / CEM pool / reactivation
                            let owns_order = self
                                .strategy_engine
                                .owns_client_order_id(&order.client_order_id);

                            let is_preemption_cancel = if owns_order
                                && order.state.to_lowercase() == "canceled"
                            {
                                if let Some(cem) = self.pipeline.cem_manager() {
                                    cem.is_planned_preemption_cancel(&order.client_order_id).await
                                } else {
                                    false
                                }
                            } else {
                                false
                            };

                            if is_preemption_cancel {
                                log::info!(
                                    "[Bot #{}] 🅿️ [CEM Preemption] Order {} was cancelled by CEM preemption plan. Notifying strategy to move to PARKED.",
                                    bot_id, order.client_order_id
                                );
                                self.strategy_engine.on_cem_preemption_canceled(&order.client_order_id);
                            }

                            // เมื่อ Order มีสถานะ Terminal (filled หรือ canceled) -> ปลดล็อกเงินใน CEM ทันที
                            // เพื่อให้เมื่อ Strategy ส่ง Action วาง Order คืน (Self-Healing) ทุนจะพร้อมใช้งานทันที
                            // ไม่เกิด deficit หลอกๆ จากการที่ทุนเก่ายังไม่ถูกปลด
                            let state_str = order.state.to_lowercase();
                            let is_terminal = state_str == "filled" || state_str == "canceled";
                            if is_terminal {
                                if owns_order {
                                    self.pipeline
                                        .clear_dedup(&bot_id, &account_id, &pair, &order.client_order_id)
                                        .await;
                                }
                                if let Some(cem) = self.pipeline.cem_manager() {
                                    if owns_order {
                                        cem.remove_preemption_cancel(&order.client_order_id).await;
                                    }
                                    if let Some((asset, amount, parked)) =
                                        cem.complete_terminal_order(&order.client_order_id).await
                                    {
                                        if parked {
                                            log::info!(
                                                "🅿️ [CEM] Preempted order {} parked in CEM pool — {} {} released, waiting for reactivation",
                                                order.client_order_id, amount, asset
                                            );
                                        } else {
                                            log::info!(
                                                "🔓 [CEM] Released {} {} from terminal order {} (state={})",
                                                amount, asset, order.client_order_id, order.state
                                            );
                                        }
                                    }
                                }
                            } else {
                                // order ยังไม่ terminal (live/partially_filled) -> ยืนยันกับ CEM ว่ามีตัวตน
                                if let Some(cem) = self.pipeline.cem_manager() {
                                    cem.mark_order_live(&order.client_order_id).await;
                                }
                            }

                            let actions = self.strategy_engine.on_order_update(&order);
                            self.process_strategy_actions(&bot_id, &account_id, &pair, actions)
                                .await;

                            // ปลุก Parked Orders ตาม priority หรือ trigger preemption ถ้ามี order เพิ่ง confirm live
                            self.reevaluate_and_route().await;

                            // The OKX orders channel includes the latest execution on each
                            // fill update. Route it through the same strategy/persistence path.
                            if let Some(fill) = Self::fill_from_order_update(&order) {
                                self.process_fill_update(&fill, &bot_id, &account_id, &pair).await;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            log::warn!("[Bot #{}] ⚠️ Order stream lagged by {} messages", bot_id, skipped);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            log::debug!("[Bot #{}] Order stream closed", bot_id);
                        }
                    }
                }

                // 4. รับอัปเดตการจับคู่เทรดจริง MyTrades / Fills (Private WS)
                fill_result = trades_rx.recv() => {
                    match fill_result {
                        Ok(fill) => {
                            self.process_fill_update(&fill, &bot_id, &account_id, &pair).await;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            log::warn!("[Bot #{}] ⚠️ MyTrades stream lagged by {} messages", bot_id, skipped);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            log::debug!("[Bot #{}] MyTrades stream closed", bot_id);
                        }
                    }
                }

                // 5. รับอัปเดตยอดเงินคงเหลือจาก Account Balance WebSocket
                balance_result = balance_rx.recv() => {
                    match balance_result {
                        Ok(balance_update) => {
                            if let Some(cem) = self.pipeline.cem_manager() {
                                for detail in balance_update.details {
                                    let avail_bal = Decimal::from_str(&detail.avail_bal).unwrap_or(Decimal::ZERO);
                                    let total_eq = Decimal::from_str(&detail.eq).unwrap_or(avail_bal);
                                    cem.update_balance_from_ws(&detail.ccy, avail_bal, total_eq).await;
                                }

                                // หลังอัปเดต balance ใหม่ -> ปลุกตาม priority แล้ว route กลับเจ้าของ
                                self.reevaluate_and_route().await;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            log::warn!("[Bot #{}] ⚠️ Balance stream lagged by {} messages", bot_id, skipped);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            log::debug!("[Bot #{}] Balance stream closed", bot_id);
                        }
                    }
                }

                // 7. รับ Reactivated Order ที่ CEM route กลับมาให้วางใหม่ (บอทเจ้าของเท่านั้น)
                reactivation = reactivation_rx.recv() => {
                    if let Some(r_order) = reactivation {
                        let actions = self.strategy_engine.on_cem_reactivate(&r_order);
                        if actions.is_empty() {
                            // Strategy ไม่รู้จักคำสั่งนี้ (เช่น รีสตาร์ทบอทแล้ว strategy state หาย)
                            // -> เก็บกวาด entry + คืนเงินจอง กันเงินถูกจองค้างตาย
                            if let Some(cem) = self.pipeline.cem_manager() {
                                if let Some((asset, amount)) = cem.abandon_reactivated_order(&r_order.intent_id).await {
                                    log::warn!(
                                        "[Bot #{}] ⚠️ Reactivation abandoned for {} (strategy does not track it) — {} {} returned to free",
                                        bot_id, r_order.intent_id, amount, asset
                                    );
                                }
                            }
                        } else {
                            self.process_strategy_actions(&bot_id, &account_id, &pair, actions).await;
                        }
                    }
                }

                // 8. Trading op ack ระดับ item (เช่น cancel 51400 = order ไม่มีอยู่บน exchange แล้ว,
                //    place sCode!=0 เช่น 51008 = order ไม่เคยถูกสร้างเลย)
                //    order ที่ place ถูก reject จะไม่มี terminal event มาเอง -> ต้อง reconcile ด้วย ack นี้
                //    (เคสจริง zzz.log 03:00:00: cancel preempt ของ reactivation ที่ไม่เคยถึง OKX -> 51400;
                //     app_2026-10-13: place ของ BOT4d81... ถูก 51008 หลัง dispatch -> order ผี + dedup ค้าง -> ลูป)
                ack_result = op_ack_rx.recv() => {
                    match ack_result {
                        Ok(ack) => {
                            let place_rejected = ack.op == "order";
                            let order_gone = ack.op.contains("cancel")
                                && matches!(ack.s_code.as_str(), "51400" | "51603");

                            if place_rejected {
                                // เฉพาะ executor ของบอทเจ้าของ clOrdId เท่านั้น:
                                // dedup cache และ strategy state เป็น instance ต่อ bot
                                if self
                                    .strategy_engine
                                    .owns_client_order_id(&ack.cl_ord_id)
                                {
                                    // ล้าง dedup (ไม่งั้น clOrdId ถูก block ตลอดกาล) + คืน lock ของ CEM
                                    // + ตั้ง backoff ห้าม reevaluate ปลุกซ้ำทันที (กันลูป 65 รอบ/วินาที)
                                    let released = self
                                        .pipeline
                                        .handle_place_reject(
                                            &bot_id, &account_id, &pair, &ack.cl_ord_id,
                                        )
                                        .await;
                                    match released {
                                        Some((asset, amount)) => log::warn!(
                                            "[Bot #{}] ❌ Place rejected by OKX: clOrdId={} code={} msg='{}' — order never reached exchange; {} {} returned to CEM, retry delayed",
                                            bot_id, ack.cl_ord_id, ack.s_code, ack.s_msg, amount, asset
                                        ),
                                        None => log::warn!(
                                            "[Bot #{}] ❌ Place rejected by OKX: clOrdId={} code={} msg='{}' — order never reached exchange",
                                            bot_id, ack.cl_ord_id, ack.s_code, ack.s_msg
                                        ),
                                    }
                                    // Strategy ต้องเลิกถือว่า order นี้อยู่บน exchange (ไม่งั้น chase -> 51503
                                    // และไม่มีใคร rebalance ฝั่งนี้) -> ย้ายกลับเป็น PARKED
                                    self.strategy_engine
                                        .on_place_rejected(&ack.cl_ord_id, &ack.s_msg);
                                    self.reevaluate_and_route().await;
                                }
                            } else if order_gone {
                                // CancelPending -> Parked + คืน lock (idempotent:
                                // ถ้า terminal event มาทำไปแล้วจะเป็น no-op)
                                let owns = self.strategy_engine.owns_client_order_id(&ack.cl_ord_id);
                                if owns {
                                    self.pipeline
                                        .clear_dedup(&bot_id, &account_id, &pair, &ack.cl_ord_id)
                                        .await;
                                }
                                if let Some((asset, amount, parked)) = self
                                    .pipeline
                                    .handle_cancel_gone(&bot_id, &account_id, &pair, &ack.cl_ord_id)
                                    .await
                                {
                                    if parked {
                                        log::info!(
                                            "[Bot #{}] 🅿️ [CEM] Cancel ack {} ({}) — order not on exchange; {} {} returned, waiting for reactivation",
                                            bot_id, ack.cl_ord_id, ack.s_code, amount, asset
                                        );
                                    } else {
                                        log::info!(
                                            "[Bot #{}] 🔓 [CEM] Cancel ack {} ({}) — released {} {} from order no longer on exchange",
                                            bot_id, ack.cl_ord_id, ack.s_code, amount, asset
                                        );
                                    }
                                    self.reevaluate_and_route().await;
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            log::warn!("[Bot #{}] ⚠️ Op-ack stream lagged by {} messages", bot_id, skipped);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            log::debug!("[Bot #{}] Op-ack stream closed", bot_id);
                        }
                    }
                }

                // 6. Heartbeat ตรวจสอบความมีชีวิตของบอท
                _ = heartbeat_timer.tick() => {
                    log::debug!("[Bot #{}] 💓 Executor heartbeat is alive", bot_id);
                }
            }
        }

        // ยกเลิก route และคืนเงินจองของคำสั่งที่ยังค้างในคิว (บอทหยุดแล้ว -> ปลุกกลับเป็น Parked)
        if let Some(cem) = self.pipeline.cem_manager() {
            cem.unregister_reactivation_route(&bot_id).await;
            while let Ok(r_order) = reactivation_rx.try_recv() {
                cem.unreserve_reactivated_order(&r_order.intent_id).await;
            }
        }

        let pending_orders = self.strategy_engine.pending_cancel_orders();
        // recovery_ok = เหตุผลที่ยืนยันแล้วว่า order ของ bot นี้ไม่ live บน exchange อีกต่อไป
        // -> ค่อยปลดล็อก CEM ได้อย่างปลอดภัย (ถ้า recovery fail -> ห้ามปลด เพราะ order อาจยัง live)
        let recovery_ok = if pending_orders.is_empty() {
            true
        } else {
            log::info!(
                "[Bot #{}] 🛑 Stop cleanup: reconciling {} tracked order(s)",
                bot_id,
                pending_orders.len()
            );
            match RecoveryEngine::run_recovery(
                &bot_id,
                &pair,
                &pending_orders,
                &self.trade_client,
                self.rest_client.as_deref(),
                self.rest_credentials.as_ref(),
                &self.recovery_trades,
                self.strategy_engine.as_mut(),
            )
            .await
            {
                Ok(()) => {
                    let _ = self.persist_runtime_state(&bot_id).await;
                    true
                }
                Err(err) => {
                    log::error!(
                        "[Bot #{}] ❌ Stop cleanup recovery failed; tracked state retained: {}",
                        bot_id,
                        err
                    );
                    false
                }
            }
        };

        // เก็บกวาด CEM ของ bot ตัวเอง: ปลดล็อกปกติจะถูกทำโดย event loop ของ executor ที่ยังรันอยู่ —
        // ถ้าเป็นบอทตัวสุดท้ายของ account ไม่มีใครประมวลผล terminal event ตอนนี้ -> lock ค้าง
        // ข้าม restart (CemManager เป็น shared ต่อ account ระดับ process)
        if let Some(cem) = self.pipeline.cem_manager() {
            if recovery_ok {
                let cleaned = cem.release_bot_orders_on_stop(&bot_id).await;
                if cleaned > 0 {
                    log::info!(
                        "[Bot #{}] 🛑 Stop cleanup: released CEM state for {} order(s)",
                        bot_id,
                        cleaned
                    );
                }
            }
            // Invariant check (bug report ข้อ 5): จับ lock ลอย / free+used != total ตั้งแต่ตอน stop
            for violation in cem.invariant_violations().await {
                log::error!("[Bot #{}] ❌ CEM invariant violated at stop: {}", bot_id, violation);
            }
        }

        log::info!("[Bot #{}] ✅ Executor finished successfully", bot_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use crate::domain::strategy::fixed_ratio_rebalance::FixdRatioRebalanceConfig;

    fn mock_fixed_ratio_config() -> FixdRatioRebalanceConfig {
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
    fn converts_order_channel_execution_to_fill_event() {
        let order = OkxOrderUpdateData {
            inst_id: "BTC-USDT".to_string(),
            order_id: "ord-1".to_string(),
            client_order_id: "cl-1".to_string(),
            trade_id: "trade-1".to_string(),
            fill_size: "0.0012".to_string(),
            fill_price: "83840.7".to_string(),
            fill_time: "1790802551358".to_string(),
            fill_fee: "-0.08".to_string(),
            fill_fee_currency: "USDT".to_string(),
            exec_type: "M".to_string(),
            side: "sell".to_string(),
            ..Default::default()
        };

        let fill = BotExecutor::fill_from_order_update(&order).unwrap();
        assert_eq!(fill.trade_id, "trade-1");
        assert_eq!(fill.fill_size, "0.0012");
        assert_eq!(fill.fee, "-0.08");
        assert_eq!(fill.ts, "1790802551358");
        assert_eq!(fill.exec_type.as_deref(), Some("M"));

        let no_execution = OkxOrderUpdateData {
            fill_size: "0".to_string(),
            ..order
        };
        assert!(BotExecutor::fill_from_order_update(&no_execution).is_none());
    }

    #[tokio::test]
    async fn test_bot_executor_lifecycle_and_recovery_startup() {
        let strategy_model = StrategyModel {
            id: "test-bot-1".to_string(),
            account_id: "acc-1".to_string(),
            user_id: "user-1".to_string(),
            name: "Test Fixed Ratio".to_string(),
            pair: "BTC-USDT".to_string(),
            sandbox: true,
            status: crate::domain::strategy::StrategyStatus::Running,
            config: crate::domain::strategy::StrategyConfig::FixedRatioRebalance(mock_fixed_ratio_config()),
            state: crate::domain::strategy::StrategyState::Idle,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let public_ws = OkxPublicWsClient::new();
        let (trade_client, _rx) = OkxWsTradeClient::new(true);
        let rate_limiter = Arc::new(crate::okx::rate_limiter::OkxRateLimiter::new());
        let pipeline = OrderPipeline::new(rate_limiter, trade_client.clone());
        let cancel_token = CancellationToken::new();

        let recovery_trades = vec![
            OkxMyTradeData {
                inst_id: "BTC-USDT".to_string(),
                trade_id: "rec-trade-1".to_string(),
                order_id: "ord-1".to_string(),
                client_order_id: "cl-1".to_string(),
                fill_price: "50000.0".to_string(),
                fill_size: "0.001".to_string(),
                side: "buy".to_string(),
                fee: "0.04".to_string(),
                fee_currency: "USDT".to_string(),
                ts: "1700000000000".to_string(),
                exec_type: Some("M".to_string()),
            },
        ];

        let executor = BotExecutor::with_dependencies(
            strategy_model,
            public_ws,
            trade_client,
            pipeline,
            None,
            None,
            None,
            None,
            recovery_trades,
            cancel_token.clone(),
            Some(3),
        );

        // สั่ง cancel ทันทีเพื่อทดสอบว่า run() ผ่าน startup recovery และ exit gracefully ได้
        cancel_token.cancel();
        executor.run().await;
    }
}
