use std::time::Duration;
use tokio_util::sync::CancellationToken;

use crate::{
    domain::strategy::Strategy,
    okx::ws_client::OkxPublicWsClient,
};

/// Executor สำหรับรัน Event Loop ของบอท 1 ตัวใน Background Task
pub struct BotExecutor {
    strategy: Strategy,
    public_ws: OkxPublicWsClient,
    cancel_token: CancellationToken,
}

impl BotExecutor {
    pub fn new(
        strategy: Strategy,
        public_ws: OkxPublicWsClient,
        cancel_token: CancellationToken,
    ) -> Self {
        Self {
            strategy,
            public_ws,
            cancel_token,
        }
    }

    /// รัน Loop การทำงานของบอท (Async Event Loop)
    pub async fn run(self) {
        let bot_id = self.strategy.id.clone();
        let bot_name = self.strategy.name.clone();
        let pair = self.strategy.pair.clone();

        log::info!(
            "[Bot #{}] 🚀 Starting executor for bot '{}' (Pair: {})",
            bot_id,
            bot_name,
            pair
        );

        // ดึง Receiver สำหรับฟังราคาตลาดสด
        let mut ticker_rx = self.public_ws.subscribe_ticker();

        // สังเคราะห์ Heartbeat timer เพื่อตรวจเช็คสถานะ
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
                                log::debug!(
                                    "[Bot #{}] 📊 Ticker update for {}: Price={}",
                                    bot_id,
                                    pair,
                                    ticker.last
                                );
                                // TODO: ต่อตรรกะประเมิน Fixed Ratio Rebalance หรือ Grid ที่นี่
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

                // 3. Heartbeat ตรวจสอบความมีชีวิตของบอท
                _ = heartbeat_timer.tick() => {
                    log::debug!("[Bot #{}] 💓 Executor heartbeat is alive", bot_id);
                }
            }
        }

        log::info!("[Bot #{}] ✅ Executor finished successfully", bot_id);
    }
}
