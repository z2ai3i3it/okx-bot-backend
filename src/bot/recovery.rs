use std::{collections::HashSet, str::FromStr};

use rust_decimal::Decimal;
use thiserror::Error;

use crate::{
    bot::strategies::base::Strategy,
    okx::{
        dto::trade::{OkxMyTradeData, WsCancelOrderArg},
        rest_client::{OkxClientError, OkxRestClient, OkxRestCredentials},
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
    pub async fn cancel_tracked_orders(
        bot_id: &str,
        orders: &[WsCancelOrderArg],
        trade_client: &OkxWsTradeClient,
    ) -> Result<(), RecoveryError> {
        if orders.is_empty() {
            log::info!("[Bot #{}] 🧹 No tracked orders to cancel.", bot_id);
            return Ok(());
        }

        log::info!(
            "[Bot #{}] 🧹 Dispatching cancel for {} tracked order(s)...",
            bot_id,
            orders.len()
        );
        for cancel_arg in orders {
            let cl_ord_id = cancel_arg.client_order_id.as_deref().unwrap_or("-");
            if let Err(err) = trade_client.cancel_order_ws(cancel_arg.clone()).await {
                let message = format!(
                    "[Bot #{}] Failed to dispatch cancel for clOrdId {}: {}",
                    bot_id,
                    cl_ord_id,
                    err
                );
                log::error!("{}", message);
                return Err(RecoveryError::CancelError(message));
            }
        }

        Ok(())
    }

    /// ดำเนินการ Generic Recovery Workflow 3 ขั้นตอน:
    /// 1. Clean Slate Cancel: ยกเลิกคำสั่งคงค้างทั้งหมดของ Bot (ผ่าน Order IDs หรือ Client Order IDs ที่ระบุ)
    /// 2. Fetch Missed Fills: ดึงประวัติการ Match (Trades) ที่อาจเกิดขึ้นระหว่างออฟไลน์
    /// 3. Delegate to Strategy: ส่งต่อ Fills ให้ Strategy เป็นผู้ Replay ตัวเองผ่าน `on_recovery_reconcile`
    pub async fn run_recovery(
        bot_id: &str,
        inst_id: &str,
        pending_orders: &[WsCancelOrderArg],
        trade_client: &OkxWsTradeClient,
        rest_client: Option<&OkxRestClient>,
        rest_credentials: Option<&OkxRestCredentials>,
        missed_trades: &[OkxMyTradeData],
        strategy: &mut dyn Strategy,
    ) -> Result<(), RecoveryError> {
        log::info!(
            "[Bot #{}] 🔄 [RecoveryEngine] Starting recovery procedure for instrument '{}'...",
            bot_id,
            inst_id
        );

        // Step 1: Cancel only orders reported by this strategy.
        Self::cancel_tracked_orders(bot_id, pending_orders, trade_client).await?;

        // Step 2: Verify terminal state and retrieve fills after cancel dispatch.
        let mut recovered_trades = missed_trades.to_vec();
        if !pending_orders.is_empty() {
            let rest_client = rest_client.ok_or_else(|| {
                RecoveryError::FetchError(
                    "REST client is required to verify tracked order cancellation".to_string(),
                )
            })?;
            let rest_credentials = rest_credentials.ok_or_else(|| {
                RecoveryError::FetchError(
                    "REST credentials are required to verify tracked order cancellation".to_string(),
                )
            })?;

            for tracked_order in pending_orders {
                let cl_ord_id = tracked_order
                    .client_order_id
                    .as_deref()
                    .ok_or_else(|| RecoveryError::FetchError("Tracked order has no clOrdId".to_string()))?;
                let detail = match rest_client
                    .get_order_detail(rest_credentials, &tracked_order.inst_id, cl_ord_id)
                    .await
                {
                    Ok(detail) => detail,
                    Err(OkxClientError::ApiError { code, .. }) if code == "51603" => {
                        log::info!(
                            "[Bot #{}] Order {} is no longer present on exchange",
                            bot_id,
                            cl_ord_id
                        );
                        continue;
                    }
                    Err(err) => {
                        return Err(RecoveryError::FetchError(format!(
                            "Could not verify order {} after cancellation: {}",
                            cl_ord_id, err
                        )));
                    }
                };

                if matches!(detail.state.as_str(), "live" | "partially_filled") {
                    return Err(RecoveryError::CancelError(format!(
                        "Order {} remains {} after cancel request",
                        cl_ord_id, detail.state
                    )));
                }
                if !matches!(detail.state.as_str(), "filled" | "canceled") {
                    return Err(RecoveryError::FetchError(format!(
                        "Order {} returned unexpected state '{}'",
                        cl_ord_id, detail.state
                    )));
                }

                let expected_fill = Decimal::from_str(&detail.acc_fill_size).map_err(|err| {
                    RecoveryError::FetchError(format!(
                        "Invalid accFillSz for order {}: {}",
                        cl_ord_id, err
                    ))
                })?;
                if expected_fill <= Decimal::ZERO {
                    continue;
                }
                if detail.order_id.is_empty() {
                    return Err(RecoveryError::FetchError(format!(
                        "Order {} has accFillSz={} but no ordId",
                        cl_ord_id, expected_fill
                    )));
                }

                let fills = rest_client
                    .get_order_fills(rest_credentials, &tracked_order.inst_id, &detail.order_id)
                    .await
                    .map_err(|err| {
                        RecoveryError::FetchError(format!(
                            "Could not fetch fills for order {}: {}",
                            cl_ord_id, err
                        ))
                    })?;
                let recovered_size = fills.iter().try_fold(Decimal::ZERO, |total, fill| {
                    Decimal::from_str(&fill.fill_size).map(|size| total + size)
                }).map_err(|err| {
                    RecoveryError::FetchError(format!("Invalid fill size from REST: {}", err))
                })?;
                if fills.is_empty() || recovered_size < expected_fill {
                    return Err(RecoveryError::FetchError(format!(
                        "Order {} reports accFillSz={} but REST fills total {}",
                        cl_ord_id, expected_fill, recovered_size
                    )));
                }
                recovered_trades.extend(fills);
            }
        }

        let mut seen = HashSet::new();
        recovered_trades.retain(|trade| {
            let key = if trade.trade_id.is_empty() {
                format!("{}:{}:{}:{}", trade.order_id, trade.ts, trade.fill_price, trade.fill_size)
            } else {
                trade.trade_id.clone()
            };
            seen.insert(key)
        });
        recovered_trades.sort_by(|left, right| {
            left.ts
                .parse::<u64>()
                .unwrap_or(0)
                .cmp(&right.ts.parse::<u64>().unwrap_or(0))
                .then_with(|| left.trade_id.cmp(&right.trade_id))
        });

        // Step 3: Delegate strategy-specific fill replay.
        log::info!(
            "[Bot #{}] 📥 [RecoveryEngine] Step 2: Reconciling with {} missed fill trade(s)...",
            bot_id,
            recovered_trades.len()
        );

        // Strategy-specific recovery owns restoring its persisted runtime state.
        strategy
            .recover(&recovered_trades)
            .await
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
    use crate::okx::ws_trade::WsTradingOp;

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
    async fn cancel_tracked_orders_queues_only_owned_cancel_args() {
        let (trade_client, mut op_rx) = OkxWsTradeClient::new(true);
        let order = WsCancelOrderArg {
            inst_id: "BTC-USDT".to_string(),
            inst_id_code: Some(3),
            order_id: None,
            client_order_id: Some("owned-by-this-bot".to_string()),
        };

        RecoveryEngine::cancel_tracked_orders("bot-123", &[order], &trade_client)
            .await
            .unwrap();

        match op_rx.recv().await {
            Some(WsTradingOp::CancelOrder(arg)) => {
                assert_eq!(arg.client_order_id.as_deref(), Some("owned-by-this-bot"));
                assert_eq!(arg.inst_id_code, Some(3));
            }
            other => panic!("Expected tracked-order cancel operation, got {other:?}"),
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
            &[],
            &trade_client,
            None,
            None,
            &missed_trades,
            &mut strategy,
        )
        .await;

        assert!(result.is_ok());
        assert_eq!(strategy.reconciled_trades_count, 2);
    }
}
