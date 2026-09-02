
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetAnchorOrder {
    pub side: Side,
    pub price_qb: Decimal,
    pub amount_b: Decimal,
}

/// record เดียวกับที่ python คืนเป็น dict แล้วเอาไป insert DB
/// ที่นี่คือ record ที่จะถูก append ลง /logs/{bot_name}.json (ไฟล์แยกตาม bot)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rebalance {
    pub id: String, // db id
    #[serde(rename = "ordId")] // ref trade id from OKX
    pub order_id: String,
    #[serde(rename = "clOrdId")] // ref trade client id from OKX
    pub cl_ord_id: String,
    pub sequence: u64,
    pub symbol: String,
    #[serde(rename = "type")]
    pub order_type: String,
    pub side: String,
    pub fee_type: String,
    pub target_anchor_buy: Option<TargetAnchorOrder>,
    pub target_anchor_sell: Option<TargetAnchorOrder>,
    pub price_qb: Decimal,
    pub base_balance_before_b: Decimal,
    pub base_value_before_q: Decimal,
    pub quote_value_before_q: Decimal,
    pub port_value_before_q: Decimal,
    pub got_b: Decimal,
    pub net_base_balance_b: Decimal,
    pub base_value_after_q: Decimal,
    pub quote_value_after_q: Decimal,
    pub port_value_after_q: Decimal,
    pub base_additional_b: Decimal,
    pub quote_additional_q: Decimal,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

impl Rebalance {
    pub fn new(
        id: String,
        cl_ord_id: String,
        sequence: u64,
        symbol: String,
        order_type: String,
        side: String,
        fee_type: String,
        target_anchor_buy: Option<TargetAnchorOrder>,
        target_anchor_sell: Option<TargetAnchorOrder>,
        price_qb: Decimal,
        base_balance_before_b: Decimal,
        base_value_before_q: Decimal,
        quote_value_before_q: Decimal,
        port_value_before_q: Decimal,
        got_b: Decimal,
        net_base_balance_b: Decimal,
        base_value_after_q: Decimal,
        quote_value_after_q: Decimal,
        port_value_after_q: Decimal,
        base_additional_b: Decimal,
        quote_additional_q: Decimal,
        created_at: String,
    ) -> Self {
        Self {
        }
    }
}
