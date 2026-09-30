use std::time::Duration;
use tokio_util::sync::CancellationToken;

use crate::{
    bot::strategies::{base::Strategy, fixed_ratio::FixedRatioStrategy},
    domain::strategy::Strategy as StrategyModel,
    okx::{ws_client::OkxPublicWsClient, ws_trade::OkxWsTradeClient},
    pipeline::order_pipeline::OrderPipeline,
};

/// Executor สำหรับรัน Event Loop ของบอท 1 ตัวใน Background Task
pub struct BotExecutor {
    strategy_model: StrategyModel,
    strategy_engine: Box<dyn Strategy>,
    public_ws: OkxPublicWsClient,
    trade_client: OkxWsTradeClient,
    pipeline: OrderPipeline,
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
        // สร้าง Engine ตามประเภทของ Strategy (ใน Phase นี้เริ่มจาก FixedRatioStrategy Mock)
        let strategy_engine: Box<dyn Strategy> = Box::new(FixedRatioStrategy::new(
            strategy_model.id.clone(),
            strategy_model.pair.clone(),
            "0.00001".to_string(), // fixed size สำหรับทดสอบ
        ));

        Self {
            strategy_model,
            strategy_engine,
            public_ws,
            trade_client,
            pipeline,
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
