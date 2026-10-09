use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::{
    okx::rate_limiter::OrderOpType,
    pipeline::context::PipelineContext,
};

/// Step 1: Dedup Step - ป้องกันการยิงคำสั่งที่มี clOrdId ซ้ำเข้าสู่ระบบ
#[derive(Clone, Default)]
pub struct DedupStep {
    seen_place_order_ids: Arc<Mutex<HashSet<(String, String, String, String)>>>,
}

impl DedupStep {
    pub fn new() -> Self {
        Self {
            seen_place_order_ids: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub async fn execute(&self, ctx: &mut PipelineContext) -> Result<(), String> {
        if ctx.op_type != OrderOpType::PlaceSingle {
            return Ok(());
        }

        let key = (
            ctx.bot_id.clone(),
            ctx.account_id.clone(),
            ctx.inst_id.clone(),
            ctx.client_order_id.clone(),
        );
        let mut seen = self.seen_place_order_ids.lock().await;

        if !seen.insert(key) {
            let reason = format!(
                "Duplicate place-order client_order_id '{}' detected by DedupStep",
                ctx.client_order_id
            );
            log::warn!("[Bot #{}] ⚠️ {}", ctx.bot_id, reason);
            ctx.reject(reason.clone());
            return Err(reason);
        }

        log::debug!(
            "[Bot #{}] ✅ Place-order DedupStep passed for clOrdId: {}",
            ctx.bot_id,
            ctx.client_order_id
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::DedupStep;
    use crate::{
        bot::strategies::base::StrategyAction,
        okx::dto::trade::{WsCancelOrderArg, WsPlaceOrderArg},
        pipeline::context::PipelineContext,
    };

    fn place_context() -> PipelineContext {
        PipelineContext::new(
            "bot-1".to_string(),
            "account-1".to_string(),
            "BTC-USDT".to_string(),
            StrategyAction::PlaceOrder {
                arg: WsPlaceOrderArg {
                    id: None,
                    inst_id: "BTC-USDT".to_string(),
                    inst_id_code: None,
                    td_mode: "cash".to_string(),
                    side: "buy".to_string(),
                    ord_type: "post_only".to_string(),
                    sz: "0.001".to_string(),
                    px: Some("50000".to_string()),
                    client_order_id: Some("cl-order-1".to_string()),
                    tag: None,
                },
                reference_bid: None,
                reference_ask: None,
            },
        )
        .unwrap()
    }

    #[tokio::test]
    async fn rejects_duplicate_place_but_allows_cancel_with_same_client_order_id() {
        let dedup = DedupStep::new();
        let mut first_place = place_context();
        dedup.execute(&mut first_place).await.unwrap();

        let mut duplicate_place = place_context();
        assert!(dedup.execute(&mut duplicate_place).await.is_err());

        let mut cancel = PipelineContext::new(
            "bot-1".to_string(),
            "account-1".to_string(),
            "BTC-USDT".to_string(),
            StrategyAction::CancelOrder(WsCancelOrderArg {
                inst_id: "BTC-USDT".to_string(),
                inst_id_code: None,
                order_id: None,
                client_order_id: Some("cl-order-1".to_string()),
            }),
        )
        .unwrap();
        dedup.execute(&mut cancel).await.unwrap();
    }
}
