use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Fixed ratio rebalance strategy configuration
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FixdRatioRebalanceConfig {
    #[schema(value_type = String, example = "0.1")]
    pub initial_capital_b: Decimal,
    #[schema(value_type = String, example = "5000.0")]
    pub initial_capital_q: Decimal,
    #[schema(value_type = String, example = "0.5")]
    pub base_value_ratio_target: Decimal,
    #[schema(value_type = String, example = "0.52")]
    pub sell_threshold_ratio_target: Decimal,
    #[schema(value_type = String, example = "0.48")]
    pub buy_threshold_ratio_target: Decimal,
    #[schema(value_type = String, example = "0.0008")]
    pub maker_fee: Decimal,
    #[schema(value_type = String, example = "0.001")]
    pub taker_fee: Decimal,
    #[schema(value_type = String, example = "0.0001")]
    pub min_size_b: Decimal,
    #[schema(value_type = String, example = "1.0")]
    pub decimal_place_price_qb: Decimal,
    #[schema(value_type = String, example = "0.0001")]
    pub decimal_place_amount_b: Decimal,
    #[schema(value_type = String, example = "0.01")]
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
