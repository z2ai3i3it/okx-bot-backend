use crate::{
    bot::strategies::base::StrategyAction,
    okx::rate_limiter::OrderOpType,
};

/// สถานะการประมวลผลคำสั่งใน Pipeline
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineStatus {
    Pending,
    Passed,
    Rejected(String),
}

/// ข้อมูล Context ที่ส่งผ่านแต่ละ Step บนสายพาน Order Pipeline
#[derive(Debug, Clone)]
pub struct PipelineContext {
    pub bot_id: String,
    pub account_id: String,
    pub inst_id: String,
    pub client_order_id: String,
    pub op_type: OrderOpType,
    pub action: StrategyAction,
    pub status: PipelineStatus,
    pub cancel_plan: Option<crate::pipeline::simulate_cancel_plan::CancelPlanResult>,
}

impl PipelineContext {
    pub fn new(
        bot_id: String,
        account_id: String,
        inst_id: String,
        action: StrategyAction,
    ) -> Result<Self, String> {
        let (client_order_id, op_type) = match &action {
            StrategyAction::PlaceOrder { arg, .. } => {
                let cl_ord_id = arg
                    .client_order_id
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                (cl_ord_id, OrderOpType::PlaceSingle)
            }
            StrategyAction::CancelOrder(arg) => {
                let cl_ord_id = arg
                    .client_order_id
                    .clone()
                    .or_else(|| arg.order_id.clone())
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                (cl_ord_id, OrderOpType::Cancel)
            }
            StrategyAction::AmendOrder(arg) => {
                let cl_ord_id = arg
                    .client_order_id
                    .clone()
                    .or_else(|| arg.order_id.clone())
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                (cl_ord_id, OrderOpType::Amend)
            }
            StrategyAction::Hold => return Err("Cannot build pipeline context for Hold action".to_string()),
        };

        Ok(Self {
            bot_id,
            account_id,
            inst_id,
            client_order_id,
            op_type,
            action,
            status: PipelineStatus::Pending,
            cancel_plan: None,
        })
    }

    pub fn is_rejected(&self) -> bool {
        matches!(self.status, PipelineStatus::Rejected(_))
    }

    pub fn reject(&mut self, reason: String) {
        self.status = PipelineStatus::Rejected(reason);
    }
}
