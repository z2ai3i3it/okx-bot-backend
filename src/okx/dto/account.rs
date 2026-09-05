use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// โครงสร้างข้อมูล Balance ย่อยแยกรายเหรียญ
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct OkxBalanceDetail {
    /// สกุลเงิน เช่น "USDT", "BTC"
    pub ccy: String,
    /// ยอดรวมทั้งหมด (Equity)
    #[serde(rename = "eq")]
    pub equity: String,
    /// ยอดเงินที่ใช้ได้ (Available)
    #[serde(rename = "availBal")]
    pub available_balance: String,
    /// ยอดที่ถูกตรึงหรือล็อกไว้ (Frozen)
    #[serde(rename = "frozenBal")]
    pub frozen_balance: String,
}

/// ข้อมูล Asset Balance รวมของ Account
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct OkxAccountBalanceData {
    /// ยอดรวมพอร์ตแปลงเป็น USD
    #[serde(rename = "totalEq")]
    pub total_equity_usd: String,
    /// รายละเอียดแยกตามเหรียญ
    #[serde(default)]
    pub details: Vec<OkxBalanceDetail>,
}

/// Generic OKX v5 API Response Wrapper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OkxApiResponse<T> {
    /// รหัสสถานะ ("0" คือสำเร็จ)
    pub code: String,
    /// ข้อความแจ้งเตือนหรือ Error
    pub msg: String,
    /// ข้อมูลผลลัพธ์
    #[serde(default)]
    pub data: Vec<T>,
}

/// ผลลัพธ์จากการยิง Verify Account ผ่าน REST Client
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AccountVerificationResult {
    pub is_valid: bool,
    pub message: String,
    pub total_equity_usd: Option<String>,
    pub balances: Vec<OkxBalanceDetail>,
}
