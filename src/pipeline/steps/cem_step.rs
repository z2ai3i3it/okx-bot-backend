use crate::pipeline::context::PipelineContext;

/// Step 2: CEM Gatekeeper (Capital Efficiency Manager)
/// ใน Phase ทดสอบนี้ ทำงานเป็น Pass-through / Mock:
/// - รับคำสั่งเข้ามา ตรวจสอบความถูกต้องเบื้องต้น
/// - ปล่อยผ่าน (Pass) เพื่อให้ flow การส่งคำสั่งทดสอบทำงานได้ครบวงจร
/// - รองรับการเสียบ logic simulate cancel plan และ balance allocation ในอนาคต
#[derive(Clone, Default)]
pub struct CemStep;

impl CemStep {
    pub fn new() -> Self {
        Self
    }

    pub async fn execute(&self, ctx: &mut PipelineContext) -> Result<(), String> {
        log::info!(
            "[Bot #{}] 🛡️ [CEM Gatekeeper] Pass-through check approved for clOrdId: {} (Op: {:?})",
            ctx.bot_id,
            ctx.client_order_id,
            ctx.op_type
        );
        Ok(())
    }
}
