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

impl FixdRatioRebalanceConfig {
    pub fn new(
        initial_capital_b: Decimal,
        initial_capital_q: Decimal,
        base_value_ratio_target: Decimal,
        sell_threshold_ratio_target: Decimal,
        buy_threshold_ratio_target: Decimal,
        maker_fee: Decimal,
        taker_fee: Decimal,
        min_size_b: Decimal,
        decimal_place_price_qb: Decimal,
        decimal_place_amount_b: Decimal,
        decimal_place_total_q: Decimal,
    ) -> Self {
        Self {
            initial_capital_b,
            initial_capital_q,
            base_value_ratio_target,
            sell_threshold_ratio_target,
            buy_threshold_ratio_target,
            maker_fee,
            taker_fee,
            min_size_b,
            decimal_place_price_qb,
            decimal_place_amount_b,
            decimal_place_total_q,
        }
    }
}
