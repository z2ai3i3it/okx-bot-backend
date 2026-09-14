use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum Side {
    Buy = 0,
    Sell = 1,
}

impl Side {
    pub fn as_str(&self) -> &'static str {
        match self {
            Side::Buy => "buy",
            Side::Sell => "sell",
        }
    }
}

/// Order ที่กำลังถูกติดตามการประมวลผลของ Strategy
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TrackedOrder {
    #[schema(example = "cl-ord-1234")]
    pub cl_ord_id: String,
    pub side: Side,
    #[schema(value_type = String, example = "78000.0")]
    pub price: Decimal,
    #[schema(value_type = String, example = "0.001")]
    pub amount: Decimal,
}
