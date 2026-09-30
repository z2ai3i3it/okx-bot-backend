use std::sync::Arc;

use crate::{
    okx::rate_limiter::OkxRateLimiter,
    pipeline::context::PipelineContext,
};

/// Step 3: Rate Limiter Step - ตรวจสอบโควต้าและขอ Permit จาก OkxRateLimiter
#[derive(Clone)]
pub struct RateLimitStep {
    rate_limiter: Arc<OkxRateLimiter>,
}

impl RateLimitStep {
    pub fn new(rate_limiter: Arc<OkxRateLimiter>) -> Self {
        Self { rate_limiter }
    }

    pub async fn execute(&self, ctx: &mut PipelineContext) -> Result<(), String> {
        // ขอ 1 permit สำหรับคำสั่งเดี่ยว
        match self
            .rate_limiter
            .try_acquire_order(&ctx.inst_id, ctx.op_type, 1)
            .await
        {
            Ok(()) => {
                log::debug!(
                    "[Bot #{}] ⏱️ RateLimitStep permit granted for inst_id: {} op: {:?}",
                    ctx.bot_id,
                    ctx.inst_id,
                    ctx.op_type
                );
                Ok(())
            }
            Err(e) => {
                let err_msg = format!("Rate limit rejected: {}", e);
                log::warn!("[Bot #{}] ⚠️ {}", ctx.bot_id, err_msg);
                ctx.reject(err_msg.clone());
                Err(err_msg)
            }
        }
    }
}
