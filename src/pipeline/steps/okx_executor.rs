use crate::{
    bot::strategies::base::StrategyAction,
    okx::ws_trade::OkxWsTradeClient,
    pipeline::context::PipelineContext,
};

/// Step 4: OKX WS Executor - ส่งคำสั่งซื้อขายจริงผ่าน OkxWsTradeClient
#[derive(Clone)]
pub struct OkxExecutorStep {
    trade_client: OkxWsTradeClient,
}

impl OkxExecutorStep {
    pub fn new(trade_client: OkxWsTradeClient) -> Self {
        Self { trade_client }
    }

    pub async fn execute(&self, ctx: &mut PipelineContext) -> Result<(), String> {
        match &ctx.action {
            StrategyAction::PlaceOrder { arg, reference_bid, reference_ask } => {
                log::info!(
                    "[Bot #{}] 🚀 [Pipeline: OKX Executor] Dispatching PlaceOrder: clOrdId={} side={} px={:?} sz={} (Ref Bid: {:?}, Ask: {:?})",
                    ctx.bot_id,
                    arg.client_order_id.as_deref().unwrap_or("-"),
                    arg.side,
                    arg.px,
                    arg.sz,
                    reference_bid,
                    reference_ask
                );
                self.trade_client
                    .create_order_ws(arg.clone())
                    .await
                    .map_err(|e| format!("Failed to dispatch create_order_ws: {}", e))?;
            }
            StrategyAction::CancelOrder(arg) => {
                log::info!(
                    "[Bot #{}] 🛑 [Pipeline: OKX Executor] Dispatching CancelOrder: clOrdId={:?} ordId={:?}",
                    ctx.bot_id,
                    arg.client_order_id,
                    arg.order_id
                );
                self.trade_client
                    .cancel_order_ws(arg.clone())
                    .await
                    .map_err(|e| format!("Failed to dispatch cancel_order_ws: {}", e))?;
            }
            StrategyAction::AmendOrder(arg) => {
                log::info!(
                    "[Bot #{}] ✏️ [Pipeline: OKX Executor] Dispatching AmendOrder: clOrdId={:?} newPx={:?} newSz={:?}",
                    ctx.bot_id,
                    arg.client_order_id,
                    arg.new_price,
                    arg.new_size
                );
                self.trade_client
                    .edit_order_ws(arg.clone())
                    .await
                    .map_err(|e| format!("Failed to dispatch edit_order_ws: {}", e))?;
            }
            StrategyAction::Hold => {
                return Ok(());
            }
        }

        ctx.status = crate::pipeline::context::PipelineStatus::Passed;
        Ok(())
    }
}
