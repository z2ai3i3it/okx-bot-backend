use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("Invalid encryption key length: expected 32 bytes (or 64 hex characters)")]
    InvalidKeyLength,
    #[error("Failed to encrypt data: {0}")]
    EncryptionFailed(String),
    #[error("Failed to decrypt data: {0}")]
    DecryptionFailed(String),
    #[error("Invalid base64 encoding: {0}")]
    Base64Error(#[from] base64::DecodeError),
    #[error("Invalid nonce or ciphertext length")]
    InvalidPayload,
}

pub struct EncryptionService {
    key: [u8; 32],
}

impl EncryptionService {
    /// สร้าง EncryptionService จาก master key ที่เป็น Hex string (64 ตัวอักษร) หรือ raw bytes / string
    pub fn new(key_str: &str) -> Result<Self, CryptoError> {
        let key_bytes = if key_str.len() == 64 && key_str.chars().all(|c| c.is_ascii_hexdigit()) {
            let mut bytes = [0u8; 32];
            for i in 0..32 {
                bytes[i] = u8::from_str_radix(&key_str[i * 2..i * 2 + 2], 16)
                    .map_err(|_| CryptoError::InvalidKeyLength)?;
            }
            bytes
        } else if key_str.as_bytes().len() == 32 {
            let mut bytes = [0u8; 32];
            bytes.copy_from_slice(key_str.as_bytes());
            bytes
        } else {
            return Err(CryptoError::InvalidKeyLength);
        };

        Ok(Self { key: key_bytes })
    }

    /// เข้ารหัสข้อความด้วย AES-256-GCM
    /// สุ่ม Nonce 96-bit (12 bytes) แล้วรวมเข้ากับ Ciphertext แล้วแปลงเป็น Base64
    pub fn encrypt(&self, plaintext: &str) -> Result<String, CryptoError> {
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

        let nonce = Aes256Gcm::generate_nonce(&mut OsRng); // 96-bits; unique per message
        let ciphertext = cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

        // Format: [12 bytes nonce][ciphertext + tag]
        let mut combined = Vec::with_capacity(nonce.len() + ciphertext.len());
        combined.extend_from_slice(nonce.as_slice());
        combined.extend_from_slice(&ciphertext);

        Ok(BASE64.encode(combined))
    }

    /// ถอดรหัสข้อความ Base64 ด้วย AES-256-GCM
    pub fn decrypt(&self, encrypted_base64: &str) -> Result<String, CryptoError> {
        let combined = BASE64.decode(encrypted_base64)?;
        if combined.len() < 12 {
            return Err(CryptoError::InvalidPayload);
        }

        let (nonce_bytes, ciphertext) = combined.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);

        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| CryptoError::DecryptionFailed(e.to_string()))?;

        let decrypted_bytes = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| CryptoError::DecryptionFailed(e.to_string()))?;

        String::from_utf8(decrypted_bytes)
            .map_err(|e| CryptoError::DecryptionFailed(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_and_decrypt() {
        let key_str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let service = EncryptionService::new(key_str).unwrap();

        let secret = "my_super_secret_okx_api_passphrase_123!";
        let encrypted = service.encrypt(secret).unwrap();
        assert_ne!(secret, encrypted);

        let decrypted = service.decrypt(&encrypted).unwrap();
        assert_eq!(secret, decrypted);
    }

    #[test]
    fn test_unique_nonce_generates_different_ciphertexts() {
        let key_str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let service = EncryptionService::new(key_str).unwrap();

        let secret = "same_secret";
        let enc1 = service.encrypt(secret).unwrap();
        let enc2 = service.encrypt(secret).unwrap();

        assert_ne!(enc1, enc2);
        assert_eq!(service.decrypt(&enc1).unwrap(), secret);
        assert_eq!(service.decrypt(&enc2).unwrap(), secret);
    }
}
