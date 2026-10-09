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
        dto::trade::{OkxMyTradeData, OkxOrderUpdateData},
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
            .process_actions(bot_id, account_id, pair, actions)
            .await;
        for result in results {
            if let Err(err) = result {
                log::error!("[Bot #{}] ❌ Strategy pipeline action failed: {}", bot_id, err);
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

        // 3. Heartbeat timer เพื่อตรวจเช็คสถานะ
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
                            if ticker.inst_id == pair {
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
                    match order_result {
                        Ok(order) => {
                            let actions = self.strategy_engine.on_order_update(&order);
                            self.process_strategy_actions(&bot_id, &account_id, &pair, actions)
                                .await;

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

                // 5. Heartbeat ตรวจสอบความมีชีวิตของบอท
                _ = heartbeat_timer.tick() => {
                    log::debug!("[Bot #{}] 💓 Executor heartbeat is alive", bot_id);
                }
            }
        }

        let pending_orders = self.strategy_engine.pending_cancel_orders();
        if !pending_orders.is_empty() {
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
                }
                Err(err) => log::error!(
                    "[Bot #{}] ❌ Stop cleanup recovery failed; tracked state retained: {}",
                    bot_id,
                    err
                ),
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
