use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::pipeline::context::PipelineContext;

/// Step 1: Dedup Step - ป้องกันการยิงคำสั่งที่มี clOrdId ซ้ำเข้าสู่ระบบ
#[derive(Clone, Default)]
pub struct DedupStep {
    seen_client_order_ids: Arc<Mutex<HashSet<String>>>,
}

impl DedupStep {
    pub fn new() -> Self {
        Self {
            seen_client_order_ids: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub async fn execute(&self, ctx: &mut PipelineContext) -> Result<(), String> {
        let mut seen = self.seen_client_order_ids.lock().await;

        if seen.contains(&ctx.client_order_id) {
            let reason = format!(
                "Duplicate client_order_id '{}' detected by DedupStep",
                ctx.client_order_id
            );
            log::warn!("[Bot #{}] ⚠️ {}", ctx.bot_id, reason);
            ctx.reject(reason.clone());
            return Err(reason);
        }

        seen.insert(ctx.client_order_id.clone());
        log::debug!(
            "[Bot #{}] ✅ DedupStep passed for clOrdId: {}",
            ctx.bot_id,
            ctx.client_order_id
        );
        Ok(())
    }
}
