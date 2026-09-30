use chrono::Utc;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};



pub use crate::bot::math::fixed_ratio_cal::TargetAnchorOrder;

/// Record รายการ Rebalance ที่เกิดขึ้นจริงในระบบ (บันทึกลง MongoDB collection `rebalance_records`)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rebalance {
    #[serde(rename = "_id")]
    pub id: String,
    pub bot_id: String,
    #[serde(rename = "ordId")]
    pub order_id: String,
    #[serde(rename = "clOrdId")]
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
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        bot_id: String,
        order_id: String,
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
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            bot_id,
            order_id,
            cl_ord_id,
            sequence,
            symbol,
            order_type,
            side,
            fee_type,
            target_anchor_buy,
            target_anchor_sell,
            price_qb,
            base_balance_before_b,
            base_value_before_q,
            quote_value_before_q,
            port_value_before_q,
            got_b,
            net_base_balance_b,
            base_value_after_q,
            quote_value_after_q,
            port_value_after_q,
            base_additional_b,
            quote_additional_q,
            created_at: Utc::now().to_rfc3339(),
        }
    }

    /// แปลง RebalanceRecord (จาก fixed_ratio_cal) ร่วมกับ bot_id และ order_id ให้เป็น Rebalance entity สำหรับบันทึกลง DB
    pub fn from_record(
        bot_id: &str,
        order_id: &str,
        record: &crate::bot::math::fixed_ratio_cal::RebalanceRecord,
    ) -> Self {
        Self {
            id: if record.id.is_empty() {
                uuid::Uuid::new_v4().to_string()
            } else {
                record.id.clone()
            },
            bot_id: bot_id.to_string(),
            order_id: order_id.to_string(),
            cl_ord_id: record.cl_ord_id.clone(),
            sequence: record.sequence,
            symbol: record.symbol.clone(),
            order_type: record.order_type.clone(),
            side: record.side.clone(),
            fee_type: record.fee_type.clone(),
            target_anchor_buy: record.target_anchor_buy.clone(),
            target_anchor_sell: record.target_anchor_sell.clone(),
            price_qb: record.price_qb,
            base_balance_before_b: record.base_balance_before_b,
            base_value_before_q: record.base_value_before_q,
            quote_value_before_q: record.quote_value_before_q,
            port_value_before_q: record.port_value_before_q,
            got_b: record.got_b,
            net_base_balance_b: record.net_base_balance_b,
            base_value_after_q: record.base_value_after_q,
            quote_value_after_q: record.quote_value_after_q,
            port_value_after_q: record.port_value_after_q,
            base_additional_b: record.base_additional_b,
            quote_additional_q: record.quote_additional_q,
            created_at: record.created_at.clone(),
        }
    }

    /// แปลง Rebalance entity กลับเป็น RebalanceRecord สำหรับเป็น snapshot ให้กับ strategy
    pub fn to_record(&self) -> crate::bot::math::fixed_ratio_cal::RebalanceRecord {
        crate::bot::math::fixed_ratio_cal::RebalanceRecord {
            id: self.id.clone(),
            cl_ord_id: self.cl_ord_id.clone(),
            sequence: self.sequence,
            symbol: self.symbol.clone(),
            order_type: self.order_type.clone(),
            side: self.side.clone(),
            fee_type: self.fee_type.clone(),
            target_anchor_buy: self.target_anchor_buy.clone(),
            target_anchor_sell: self.target_anchor_sell.clone(),
            price_qb: self.price_qb,
            base_balance_before_b: self.base_balance_before_b,
            base_value_before_q: self.base_value_before_q,
            quote_value_before_q: self.quote_value_before_q,
            port_value_before_q: self.port_value_before_q,
            got_b: self.got_b,
            net_base_balance_b: self.net_base_balance_b,
            base_value_after_q: self.base_value_after_q,
            quote_value_after_q: self.quote_value_after_q,
            port_value_after_q: self.port_value_after_q,
            base_additional_b: self.base_additional_b,
            quote_additional_q: self.quote_additional_q,
            created_at: self.created_at.clone(),
        }
    }
}
