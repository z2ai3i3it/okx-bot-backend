use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Grid strategy configuration (temporary)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GridConfig {
    #[schema(value_type = String, example = "50.0")]
    pub grid_size: Decimal,
    #[schema(example = 10)]
    pub num_grids: u64,
}

impl GridConfig {
    pub fn new(grid_size: Decimal, num_grids: u64) -> Self {
        Self {
            grid_size,
            num_grids,
        }
    }
}
