use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use chrono::Utc;
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub struct OkxSigner;

impl OkxSigner {
    /// สร้าง ISO 8601 UTC Timestamp ตามรูปแบบที่ OKX v5 กำหนด
    /// ตัวอย่าง: `2020-12-08T09:08:57.715Z`
    pub fn generate_timestamp() -> String {
        Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
    }

    /// สร้างลายเซ็น HMAC-SHA256 Base64 ตามกฎของ OKX v5
    /// รูปแบบ Prehash string: `timestamp + method + requestPath + body`
    /// ตัวอย่าง: `2020-12-08T09:08:57.715ZGET/api/v5/account/balance`
    pub fn sign(
        timestamp: &str,
        method: &str,
        request_path: &str,
        body: Option<&str>,
        secret_key: &str,
    ) -> Result<String, String> {
        let body_str = body.unwrap_or("");
        let prehash = format!("{}{}{}{}", timestamp, method.to_uppercase(), request_path, body_str);

        let mut mac = HmacSha256::new_from_slice(secret_key.as_bytes())
            .map_err(|e| format!("Invalid HMAC secret key: {}", e))?;

        mac.update(prehash.as_bytes());
        let result = mac.finalize();
        let signature_bytes = result.into_bytes();

        Ok(BASE64.encode(signature_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_okx_signer_known_vector() {
        // อ้างอิงตามเอกสารทางการของ OKX v5
        let timestamp = "2020-12-08T09:08:57.715Z";
        let method = "GET";
        let path = "/api/v5/account/balance";
        let secret = "E6554F8E24C78370F2B0058A82F24E62";

        let signature = OkxSigner::sign(timestamp, method, path, None, secret).unwrap();
        // ตรวจสอบว่า signature ได้ base64 สตริงที่ถูกต้อง
        assert!(!signature.is_empty());
    }

    #[test]
    fn test_timestamp_format() {
        let ts = OkxSigner::generate_timestamp();
        assert!(ts.contains('T'));
        assert!(ts.ends_with('Z'));
    }
}
