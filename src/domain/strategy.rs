use rust_decimal::Decimal;

/// Fixed ratio rebalance strategy configuration
pub struct FixdRatioRebalanceConfig {
    pub initial_capital_b: Decimal,
    pub initial_capital_q: Decimal,
    pub base_value_ratio_target: Decimal,
    pub sell_threshold_ratio_target: Decimal,
    pub buy_threshold_ratio_target: Decimal,
    pub maker_fee: Decimal,
    pub taker_fee: Decimal,
    pub min_size_b: Decimal,
    pub decimal_place_price_qb: Decimal,
    pub decimal_place_amount_b: Decimal,
    pub decimal_place_total_q: Decimal,
}

/// Grid strategy configuration (temporary)
pub struct GridConfig {
    pub grid_size: Decimal,
    pub num_grids: u64,
}

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
