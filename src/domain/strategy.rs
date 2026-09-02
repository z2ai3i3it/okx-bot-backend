use rust_decimal::Decimal;

use crate::domain::strategy::fixed_ratio_rebalance::FixdRatioRebalanceConfig;
use crate::domain::strategy::grid::GridConfig;


/// Strategy configuration
enum StrategyConfig {
    FixedRatioRebalance(FixdRatioRebalanceConfig),
    Grid(GridConfig),
}

/// Strategy
pub struct Strategy {
    pub id: String,
    pub name: String,
    pub pair: String,
    pub api_key: String,
    pub secret_key: String,
    pub passphrase: String,
    pub sandbox: bool,
    pub config: StrategyConfig,
    pub tracked_buy: Option<TrackedOrder>,
    pub tracked_sell: Option<TrackedOrder>,
    pub status: String,
}
