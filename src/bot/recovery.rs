use thiserror::Error;

use crate::{
    bot::strategies::base::Strategy,
    okx::{
        dto::trade::{OkxMyTradeData, WsCancelOrderArg},
        ws_trade::OkxWsTradeClient,
    },
};

#[derive(Debug, Error)]
pub enum RecoveryError {
    #[error("Strategy reconcile error: {0}")]
    StrategyError(String),

    #[error("Cancellation error: {0}")]
    CancelError(String),

    #[error("Fetch fills error: {0}")]
    FetchError(String),
}

/// Generic Recovery Engine (Layer 7)
/// รับผิดชอบการทำ Reconcile State ของบอทก่อนเริ่มเข้าสู่ Live Event Loop
/// ออกแบบเป็น Generic โดยไม่ผูกกับ Rebalance หรือกลยุทธ์เฉพาะเจาะจงใดๆ
pub struct RecoveryEngine;

impl RecoveryEngine {
    /// ดำเนินการ Generic Recovery Workflow 3 ขั้นตอน:
    /// 1. Clean Slate Cancel: ยกเลิกคำสั่งคงค้างทั้งหมดของ Bot (ผ่าน Order IDs หรือ Client Order IDs ที่ระบุ)
    /// 2. Fetch Missed Fills: ดึงประวัติการ Match (Trades) ที่อาจเกิดขึ้นระหว่างออฟไลน์
    /// 3. Delegate to Strategy: ส่งต่อ Fills ให้ Strategy เป็นผู้ Replay ตัวเองผ่าน `on_recovery_reconcile`
    pub async fn run_recovery(
        bot_id: &str,
        inst_id: &str,
        pending_client_order_ids: &[String],
        trade_client: &OkxWsTradeClient,
        missed_trades: &[OkxMyTradeData],
        strategy: &mut dyn Strategy,
    ) -> Result<(), RecoveryError> {
        log::info!(
            "[Bot #{}] 🔄 [RecoveryEngine] Starting recovery procedure for instrument '{}'...",
            bot_id,
            inst_id
        );

        // Step 1: Clean Slate Cancel
        if !pending_client_order_ids.is_empty() {
            log::info!(
                "[Bot #{}] 🧹 [RecoveryEngine] Step 1: Clean slate cancelling {} pending order(s)...",
                bot_id,
                pending_client_order_ids.len()
            );
            for cl_ord_id in pending_client_order_ids {
                let cancel_arg = WsCancelOrderArg {
                    inst_id: inst_id.to_string(),
                    inst_id_code: None,
                    order_id: None,
                    client_order_id: Some(cl_ord_id.clone()),
                };
                if let Err(e) = trade_client.cancel_order_ws(cancel_arg).await {
                    log::warn!(
                        "[Bot #{}] ⚠️ [RecoveryEngine] Failed to dispatch cancel for clOrdId {}: {}",
                        bot_id,
                        cl_ord_id,
                        e
                    );
                }
            }
        } else {
            log::info!(
                "[Bot #{}] 🧹 [RecoveryEngine] Step 1: No pending orders tracked to cancel.",
                bot_id
            );
        }

        // Step 2: Log / Inspect Missed Fills
        log::info!(
            "[Bot #{}] 📥 [RecoveryEngine] Step 2: Reconciling with {} missed fill trade(s)...",
            bot_id,
            missed_trades.len()
        );

        // Step 3: Delegate to Strategy Hook
        strategy
            .on_recovery_reconcile(missed_trades)
            .map_err(RecoveryError::StrategyError)?;

        log::info!(
            "[Bot #{}] ✅ [RecoveryEngine] Recovery completed successfully for '{}'",
            bot_id,
            inst_id
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::strategies::base::StrategyAction;
    use crate::okx::dto::market::OkxTickerData;
    use crate::okx::dto::trade::OkxOrderUpdateData;

    struct DummyStrategy {
        reconciled_trades_count: usize,
    }

    impl Strategy for DummyStrategy {
        fn on_ticker(&mut self, _ticker: &OkxTickerData) -> Vec<StrategyAction> {
            Vec::new()
        }
        fn on_order_update(&mut self, _order: &OkxOrderUpdateData) -> Vec<StrategyAction> {
            Vec::new()
        }
        fn on_fill_update(&mut self, _fill: &OkxMyTradeData) -> Vec<StrategyAction> {
            Vec::new()
        }
        fn on_recovery_reconcile(
            &mut self,
            missed_trades: &[OkxMyTradeData],
        ) -> Result<(), String> {
            self.reconciled_trades_count = missed_trades.len();
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_recovery_engine_delegates_to_strategy() {
        let (trade_client, _rx) = OkxWsTradeClient::new(true);
        let mut strategy = DummyStrategy {
            reconciled_trades_count: 0,
        };

        let missed_trades = vec![
            OkxMyTradeData {
                inst_id: "BTC-USDT".to_string(),
                trade_id: "t1".to_string(),
                order_id: "o1".to_string(),
                client_order_id: "c1".to_string(),
                fill_price: "60000".to_string(),
                fill_size: "0.01".to_string(),
                side: "buy".to_string(),
                fee: "0.001".to_string(),
                fee_currency: "USDT".to_string(),
                ts: "1700000000000".to_string(),
                exec_type: Some("M".to_string()),
            },
            OkxMyTradeData {
                inst_id: "BTC-USDT".to_string(),
                trade_id: "t2".to_string(),
                order_id: "o2".to_string(),
                client_order_id: "c2".to_string(),
                fill_price: "61000".to_string(),
                fill_size: "0.01".to_string(),
                side: "sell".to_string(),
                fee: "0.001".to_string(),
                fee_currency: "USDT".to_string(),
                ts: "1700000001000".to_string(),
                exec_type: Some("M".to_string()),
            },
        ];

        let result = RecoveryEngine::run_recovery(
            "bot-123",
            "BTC-USDT",
            &["old-c1".to_string()],
            &trade_client,
            &missed_trades,
            &mut strategy,
        )
        .await;

        assert!(result.is_ok());
        assert_eq!(strategy.reconciled_trades_count, 2);
    }
}
