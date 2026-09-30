use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use crate::{
    bot::{
        recovery::RecoveryEngine,
        strategies::{base::Strategy, fixed_ratio::FixedRatioStrategy},
    },
    domain::strategy::Strategy as StrategyModel,
    okx::{dto::trade::OkxMyTradeData, ws_client::OkxPublicWsClient, ws_trade::OkxWsTradeClient},
    pipeline::order_pipeline::OrderPipeline,
    services::rebalance_service::RebalanceService,
};

/// Executor สำหรับรัน Event Loop ของบอท 1 ตัวใน Background Task
pub struct BotExecutor {
    strategy_model: StrategyModel,
    strategy_engine: Box<dyn Strategy>,
    public_ws: OkxPublicWsClient,
    trade_client: OkxWsTradeClient,
    pipeline: OrderPipeline,
    rebalance_service: Option<Arc<RebalanceService>>,
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
            Vec::new(),
            cancel_token,
        )
    }

    pub fn with_dependencies(
        strategy_model: StrategyModel,
        public_ws: OkxPublicWsClient,
        trade_client: OkxWsTradeClient,
        pipeline: OrderPipeline,
        rebalance_service: Option<Arc<RebalanceService>>,
        recovery_trades: Vec<OkxMyTradeData>,
        cancel_token: CancellationToken,
    ) -> Self {
        // สร้าง Engine ตามประเภทของ Strategy
        let strategy_engine: Box<dyn Strategy> = match &strategy_model.config {
            crate::domain::strategy::StrategyConfig::FixedRatioRebalance(cfg) => {
                Box::new(FixedRatioStrategy::new(
                    strategy_model.id.clone(),
                    strategy_model.pair.clone(),
                    cfg.clone(),
                ))
            }
            crate::domain::strategy::StrategyConfig::Grid(_) => {
                // สำหรับ Grid ให้ใช้ mock grid strategy หรือ fallback
                Box::new(crate::bot::strategies::grid::GridStrategy::new(
                    strategy_model.id.clone(),
                    strategy_model.pair.clone(),
                    "0.0001".to_string(),
                ))
            }
        };

        Self {
            strategy_model,
            strategy_engine,
            public_ws,
            trade_client,
            pipeline,
            rebalance_service,
            recovery_trades,
            cancel_token,
        }
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

        // --- Step 7.3 & 7.5: เรียก Recovery Procedure ก่อนเริ่มเปิดรับ Live Event Loop ---
        if let Err(e) = RecoveryEngine::run_recovery(
            &bot_id,
            &pair,
            &[],
            &self.trade_client,
            &self.recovery_trades,
            self.strategy_engine.as_mut(),
        )
        .await
        {
            log::warn!(
                "[Bot #{}] ⚠️ Recovery procedure encountered an issue: {}",
                bot_id,
                e
            );
        }

        // 1. ดึง Receiver สำหรับฟังราคาตลาดสด (Public WS)
        let mut ticker_rx = self.public_ws.subscribe_ticker();

        // 2. ดึง Receiver สำหรับฟัง Private Streams
        let mut orders_rx = self.trade_client.subscribe_orders();
        let mut trades_rx = self.trade_client.subscribe_my_trades();

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

                                    // ส่งต่อเข้า Order Pipeline
                                    let results = self.pipeline.process_actions(
                                        &bot_id,
                                        &account_id,
                                        &pair,
                                        actions
                                    ).await;

                                    for res in results {
                                        if let Err(err) = res {
                                            log::error!("[Bot #{}] ❌ Pipeline error: {}", bot_id, err);
                                        }
                                    }
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
                            if !actions.is_empty() {
                                let _ = self.pipeline.process_actions(
                                    &bot_id,
                                    &account_id,
                                    &pair,
                                    actions
                                ).await;
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
                            let actions = self.strategy_engine.on_fill_update(&fill);

                            // Non-blocking Database Persistence ผ่าน RebalanceService
                            if let (Some(reb_service), Some(snap)) = (
                                &self.rebalance_service,
                                self.strategy_engine.get_latest_snapshot(),
                            ) {
                                let reb_svc = reb_service.clone();
                                let b_id = bot_id.clone();
                                let ord_id = fill.order_id.clone();
                                tokio::spawn(async move {
                                    if let Err(err) = reb_svc.save_trade_fill(&b_id, &ord_id, &snap).await {
                                        log::error!(
                                            "[Bot #{}] ❌ Failed to persist rebalance snapshot to DB: {}",
                                            b_id,
                                            err
                                        );
                                    } else {
                                        log::info!(
                                            "[Bot #{}] 💾 Rebalance snapshot persisted to DB (seq={})",
                                            b_id,
                                            snap.sequence
                                        );
                                    }
                                });
                            }

                            if !actions.is_empty() {
                                let _ = self.pipeline.process_actions(
                                    &bot_id,
                                    &account_id,
                                    &pair,
                                    actions
                                ).await;
                            }
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
            recovery_trades,
            cancel_token.clone(),
        );

        // สั่ง cancel ทันทีเพื่อทดสอบว่า run() ผ่าน startup recovery และ exit gracefully ได้
        cancel_token.cancel();
        executor.run().await;
    }
}
