use rust_decimal::Decimal;

pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn as_str(&self) -> &'static str {
        match self {
            Side::Buy => "buy",
            Side::Sell => "sell",
        }
    }
}

pub struct TrackedOrder {
    pub cl_ord_id: String,
    pub side: Side,
    pub price: Decimal,
    pub amount: Decimal,
}
