use serde::{Deserialize, Serialize};

/// Payload สำหรับ Login เข้า Private WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsLoginArg {
    #[serde(rename = "apiKey")]
    pub api_key: String,
    pub passphrase: String,
    pub timestamp: String,
    pub sign: String,
}

// ==========================================
// 🔐 1. Private Streams DTOs
// ==========================================

/// 1. ข้อมูลอัปเดตสถานะคำสั่งซื้อขายจาก Private Channel `orders`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OkxOrderUpdateData {
    #[serde(rename = "instType")]
    pub inst_type: String,
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(rename = "tgtCcy")]
    pub target_currency: String,
    #[serde(rename = "ccy")]
    pub currency: String,
    #[serde(rename = "tradeQuoteCcy")]
    pub trade_quote_currency: String,
    #[serde(rename = "ordId")]
    pub order_id: String,
    #[serde(rename = "clOrdId")]
    pub client_order_id: String,
    #[serde(rename = "algoClOrdId")]
    pub algo_client_order_id: String,
    #[serde(rename = "algoId")]
    pub algo_id: String,
    pub tag: Option<String>,
    pub px: String,
    pub sz: String,
    #[serde(rename = "notionalUsd")]
    pub notional_usd: String,
    pub side: String, // "buy", "sell"
    #[serde(rename = "ordType")]
    pub ord_type: String,
    #[serde(rename = "posSide")]
    pub position_side: String,
    #[serde(rename = "tdMode")]
    pub trading_mode: String,
    #[serde(rename = "fillNotionalUsd")]
    pub fill_notional_usd: String,
    pub state: String, // "live", "partially_filled", "filled", "canceled"
    pub lever: String,
    pub pnl: String,
    #[serde(rename = "fillSz")]
    pub fill_size: String,
    #[serde(rename = "fillPx")]
    pub fill_price: String,
    #[serde(rename = "tradeId")]
    pub trade_id: String,
    #[serde(rename = "fillTime")]
    pub fill_time: String,
    #[serde(rename = "fillPnl")]
    pub fill_pnl: String,
    #[serde(rename = "fillFee")]
    pub fill_fee: String,
    #[serde(rename = "fillFeeCcy")]
    pub fill_fee_currency: String,
    #[serde(rename = "execType")]
    pub exec_type: String,
    #[serde(rename = "avgPx")]
    pub avg_price: String,
    #[serde(rename = "accFillSz")]
    pub acc_fill_size: String,
    pub fee: String,
    #[serde(rename = "feeCcy")]
    pub fee_currency: String,
    #[serde(rename = "rebateCcy")]
    pub rebate_currency: String,
    pub rebate: String,
    pub category: String,
    #[serde(rename = "uTime")]
    pub update_time: String,
    #[serde(rename = "cTime")]
    pub create_time: String,
    pub source: String,
    #[serde(rename = "reduceOnly")]
    pub reduce_only: String,
    #[serde(rename = "cancelSource")]
    pub cancel_source: String,
    #[serde(rename = "quickMgnType")]
    pub quick_margin_type: String,
    #[serde(rename = "stpId")]
    pub stp_id: String,
    #[serde(rename = "stpMode")]
    pub stp_mode: String,
    #[serde(rename = "attachAlgoClOrdId")]
    pub attached_algo_client_order_id: String,
    #[serde(rename = "lastPx")]
    pub last_price: String,
    pub outcome: String,
    pub slippage: String,
    #[serde(rename = "riskBypassResult")]
    pub risk_bypass_result: String,
    #[serde(rename = "isTpLimit")]
    pub is_tp_limit: String,
    #[serde(rename = "slTriggerPx")]
    pub sl_trigger_price: String,
    #[serde(rename = "slTriggerPxType")]
    pub sl_trigger_price_type: String,
    #[serde(rename = "tpOrdPx")]
    pub tp_order_price: String,
    #[serde(rename = "tpTriggerPx")]
    pub tp_trigger_price: String,
    #[serde(rename = "tpTriggerPxType")]
    pub tp_trigger_price_type: String,
    #[serde(rename = "slOrdPx")]
    pub sl_order_price: String,
    #[serde(rename = "fillPxVol")]
    pub fill_price_volatility: String,
    #[serde(rename = "fillPxUsd")]
    pub fill_price_usd: String,
    #[serde(rename = "fillMarkVol")]
    pub fill_mark_volatility: String,
    #[serde(rename = "fillFwdPx")]
    pub fill_forward_price: String,
    #[serde(rename = "fillMarkPx")]
    pub fill_mark_price: String,
    #[serde(rename = "fillIdxPx")]
    pub fill_index_price: String,
    #[serde(rename = "amendSource")]
    pub amend_source: String,
    #[serde(rename = "reqId")]
    pub request_id: String,
    #[serde(rename = "amendResult")]
    pub amend_result: String,
    pub code: String,
    pub msg: String,
    #[serde(rename = "pxType")]
    pub price_type: String,
    #[serde(rename = "pxUsd")]
    pub price_usd: String,
    #[serde(rename = "pxVol")]
    pub price_volatility: String,
}

#[cfg(test)]
mod order_update_tests {
    use super::OkxOrderUpdateData;

    #[test]
    fn deserializes_order_channel_fill_payload() {
        let payload = serde_json::json!({
            "instType": "SPOT",
            "instId": "BTC-USDT",
            "ordId": "3969493957075316737",
            "clOrdId": "BOTe95c66cb2e3f471a868521fce0ee8",
            "px": "83840.7",
            "sz": "0.0201",
            "side": "sell",
            "ordType": "post_only",
            "state": "partially_filled",
            "accFillSz": "0.00660418",
            "fillPx": "83840.7",
            "fillSz": "0.00119273",
            "tradeId": "1413715905",
            "fillTime": "1790802551358",
            "fillFee": "-0.0799994544888",
            "fillFeeCcy": "USDT",
            "execType": "M",
            "fee": "-0.4429592593008",
            "feeCcy": "USDT",
            "uTime": "1790802551358",
            "cTime": "1790802538624"
        });

        let order: OkxOrderUpdateData = serde_json::from_value(payload).unwrap();

        assert_eq!(order.trade_id, "1413715905");
        assert_eq!(order.fill_size, "0.00119273");
        assert_eq!(order.acc_fill_size, "0.00660418");
        assert_eq!(order.fill_fee_currency, "USDT");
    }
}

/// 2. ข้อมูลการจับคู่ซื้อขายจริง (Execution Fill) จาก Private Channel `fills` / `my_trades`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OkxMyTradeData {
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(rename = "tradeId")]
    pub trade_id: String,
    #[serde(rename = "ordId")]
    pub order_id: String,
    #[serde(rename = "clOrdId")]
    pub client_order_id: String,
    #[serde(rename = "fillPx")]
    pub fill_price: String,
    #[serde(rename = "fillSz")]
    pub fill_size: String,
    pub side: String, // "buy", "sell"
    pub fee: String,
    #[serde(rename = "feeCcy")]
    pub fee_currency: String,
    pub ts: String,
    #[serde(rename = "execType")]
    pub exec_type: Option<String>, // "T" (Taker), "M" (Maker)
}

/// 3. ข้อมูล Balance และ Margin สดจาก Private Channel `account` / `balance_and_position`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OkxAccountBalanceUpdate {
    #[serde(rename = "totalEq")]
    pub total_equity: String,
    #[serde(rename = "adjEq")]
    pub adjusted_equity: Option<String>,
    #[serde(rename = "uTime")]
    pub update_time: String,
    #[serde(default)]
    pub details: Vec<OkxBalanceDetailUpdate>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OkxBalanceDetailUpdate {
    pub ccy: String,
    pub eq: String,
    #[serde(rename = "availBal")]
    pub avail_bal: String,
    #[serde(rename = "frozenBal")]
    pub frozen_bal: String,
}

// ==========================================
// 🔧 2. Trading Operations DTOs (Request / Response)
// ==========================================

/// 1. คำสั่งยิง Single Place Order ผ่าน WebSocket Trade API (`op: "order"`)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsPlaceOrderArg {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(rename = "instIdCode", skip_serializing_if = "Option::is_none")]
    pub inst_id_code: Option<u64>,
    #[serde(rename = "tdMode")]
    pub td_mode: String, // "cash", "cross", "isolated"
    pub side: String,   // "buy", "sell"
    #[serde(rename = "ordType")]
    pub ord_type: String, // "limit", "market", "post_only"
    pub sz: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub px: Option<String>,
    #[serde(rename = "clOrdId", skip_serializing_if = "Option::is_none")]
    pub client_order_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

/// 2. คำสั่งแก้ไขออร์เดอร์ ผ่าน WebSocket (`op: "amend-order"`)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsAmendOrderArg {
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(rename = "instIdCode", skip_serializing_if = "Option::is_none")]
    pub inst_id_code: Option<u64>,
    #[serde(rename = "ordId", skip_serializing_if = "Option::is_none")]
    pub order_id: Option<String>,
    #[serde(rename = "clOrdId", skip_serializing_if = "Option::is_none")]
    pub client_order_id: Option<String>,
    #[serde(rename = "newPx", skip_serializing_if = "Option::is_none")]
    pub new_price: Option<String>,
    #[serde(rename = "newSz", skip_serializing_if = "Option::is_none")]
    pub new_size: Option<String>,
}

/// 3. คำสั่งยกเลิก Single Order ผ่าน WebSocket (`op: "cancel-order"`)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsCancelOrderArg {
    #[serde(rename = "instId")]
    pub inst_id: String,
    #[serde(rename = "instIdCode", skip_serializing_if = "Option::is_none")]
    pub inst_id_code: Option<u64>,
    #[serde(rename = "ordId", skip_serializing_if = "Option::is_none")]
    pub order_id: Option<String>,
    #[serde(rename = "clOrdId", skip_serializing_if = "Option::is_none")]
    pub client_order_id: Option<String>,
}

/// 4. คำสั่งยกเลิก Batch Cancel Orders ผ่าน WebSocket (`op: "batch-cancel-orders"`)
pub type WsBatchCancelOrderArg = Vec<WsCancelOrderArg>;

/// ผลลัพธ์การตอบรับจาก OKX เมื่อยิงคำสั่งผ่าน WebSocket
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WsOrderResult {
    #[serde(rename = "clOrdId")]
    pub client_order_id: Option<String>,
    #[serde(rename = "ordId")]
    pub order_id: Option<String>,
    #[serde(rename = "sCode")]
    pub status_code: String, // "0" สำเร็จ
    #[serde(rename = "sMsg")]
    pub status_msg: String,
    pub tag: Option<String>,
}
