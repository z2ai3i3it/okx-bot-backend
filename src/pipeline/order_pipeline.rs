use std::sync::Arc;

use crate::{
    bot::strategies::base::StrategyAction,
    okx::{rate_limiter::OkxRateLimiter, ws_trade::OkxWsTradeClient},
    pipeline::{
        context::PipelineContext,
        steps::{CemStep, DedupStep, OkxExecutorStep, RateLimitStep},
    },
};

/// สายพานประมวลผลคำสั่งซื้อขาย (Structured Concurrency Order Pipeline)
/// ลำดับขั้นตอน: Dedup -> CEM Gatekeeper -> Rate Limiter -> OKX Executor
#[derive(Clone)]
pub struct OrderPipeline {
    dedup: DedupStep,
    cem: CemStep,
    rate_limit: RateLimitStep,
    executor: OkxExecutorStep,
}

impl OrderPipeline {
    pub fn new(rate_limiter: Arc<OkxRateLimiter>, trade_client: OkxWsTradeClient) -> Self {
        Self {
            dedup: DedupStep::new(),
            cem: CemStep::new(),
            rate_limit: RateLimitStep::new(rate_limiter),
            executor: OkxExecutorStep::new(trade_client),
        }
    }

    pub fn with_cem(
        rate_limiter: Arc<OkxRateLimiter>,
        trade_client: OkxWsTradeClient,
        cem: CemStep,
    ) -> Self {
        Self {
            dedup: DedupStep::new(),
            cem,
            rate_limit: RateLimitStep::new(rate_limiter),
            executor: OkxExecutorStep::new(trade_client),
        }
    }

    /// ลำเลียงคำสั่งเดี่ยวผ่านแต่ละ Step ใน Pipeline
    pub async fn process_action(
        &self,
        bot_id: &str,
        account_id: &str,
        inst_id: &str,
        action: StrategyAction,
    ) -> Result<(), String> {
        if matches!(action, StrategyAction::Hold) {
            return Ok(());
        }

        let mut ctx = PipelineContext::new(
            bot_id.to_string(),
            account_id.to_string(),
            inst_id.to_string(),
            action,
        )?;

        // Step 1: Dedup
        self.dedup.execute(&mut ctx).await?;

        // Step 2: CEM Gatekeeper (Pass-through for now)
        self.cem.execute(&mut ctx).await?;

        // Step 3: Rate Limiter
        self.rate_limit.execute(&mut ctx).await?;

        // Step 4: OKX WS Executor
        self.executor.execute(&mut ctx).await?;

        log::info!(
            "[Bot #{}] ✨ Order pipeline completed successfully for clOrdId: {}",
            bot_id,
            ctx.client_order_id
        );

        Ok(())
    }

    /// ลำเลียงชุดคำสั่งหลายคำสั่งผ่าน Pipeline
    pub async fn process_actions(
        &self,
        bot_id: &str,
        account_id: &str,
        inst_id: &str,
        actions: Vec<StrategyAction>,
    ) -> Vec<Result<(), String>> {
        let mut results = Vec::new();
        for action in actions {
            let res = self.process_action(bot_id, account_id, inst_id, action).await;
            results.push(res);
        }
        results
    }
}
