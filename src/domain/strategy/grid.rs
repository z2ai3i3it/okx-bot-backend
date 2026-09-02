use rust_decimal::Decimal;

/// Grid strategy configuration (temporary)
pub struct GridConfig {
    pub grid_size: Decimal,
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
