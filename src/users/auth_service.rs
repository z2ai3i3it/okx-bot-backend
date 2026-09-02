use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use thiserror::Error;

use crate::domain::user::{AuthResponse, Claims, RegisterRequest, Role, User};

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("Invalid email or password")]
    InvalidCredentials,

    #[error("User already exists: {0}")]
    UserAlreadyExists(String),

    #[error("User not found")]
    UserNotFound,

    #[error("Account is suspended or inactive")]
    AccountInactive,

    #[error("Password hashing error: {0}")]
    HashError(String),

    #[error("Token generation/validation error: {0}")]
    TokenError(String),

    #[error("Validation error: {0}")]
    ValidationError(String),
}

/// การตั้งค่าสำหรับ AuthService (เช่น JWT Secret, Token Expiry)
#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub jwt_secret: String,
    /// อายุของ Token เป็นจำนวนชั่วโมง (Default: 24 ชม.)
    pub jwt_expiration_hours: i64,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            jwt_secret: "default-dev-jwt-secret-change-in-production".to_string(),
            jwt_expiration_hours: 24,
        }
    }
}

/// Service รับผิดชอบตรรกะการตรวจสอบสิทธิ์และจัดการ Token
#[derive(Debug, Clone)]
pub struct AuthService {
    config: AuthConfig,
}

impl AuthService {
    pub fn new(config: AuthConfig) -> Self {
        Self { config }
    }

    /// แฮชรหัสผ่านด้วย Argon2id พร้อมเกลือ (Salt) แบบสุ่ม
    pub fn hash_password(&self, password: &str) -> Result<String, AuthError> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        argon2
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|e| AuthError::HashError(e.to_string()))
    }

    /// ตรวจสอบรหัสผ่านที่ส่งมาว่าตรงกับ Argon2 hash หรือไม่
    pub fn verify_password(&self, password: &str, password_hash: &str) -> Result<bool, AuthError> {
        let parsed_hash = PasswordHash::new(password_hash)
            .map_err(|e| AuthError::HashError(e.to_string()))?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok())
    }

    /// ออก JWT Access Token จากข้อมูล User
    pub fn generate_token(&self, user: &User) -> Result<String, AuthError> {
        let now = Utc::now();
        let exp = (now + Duration::hours(self.config.jwt_expiration_hours)).timestamp() as usize;
        let iat = now.timestamp() as usize;

        let claims = Claims {
            sub: user.id.clone(),
            username: user.username.clone(),
            role: user.role.clone(),
            exp,
            iat,
        };

        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.config.jwt_secret.as_bytes()),
        )
        .map_err(|e| AuthError::TokenError(e.to_string()))
    }

    /// ตรวจสอบและถอดรหัส JWT Access Token ออกมาเป็น Claims
    pub fn verify_token(&self, token: &str) -> Result<Claims, AuthError> {
        decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.config.jwt_secret.as_bytes()),
            &Validation::default(),
        )
        .map(|data| data.claims)
        .map_err(|e| AuthError::TokenError(e.to_string()))
    }

    /// ประมวลผลคำขอสมัครสมาชิก (สร้าง User struct พร้อมแฮชรหัสผ่าน)
    pub fn create_user_entity(
        &self,
        id: String,
        req: RegisterRequest,
        role: Option<Role>,
    ) -> Result<User, AuthError> {
        if req.username.trim().is_empty() {
            return Err(AuthError::ValidationError("Username cannot be empty".into()));
        }
        if req.email.trim().is_empty() || !req.email.contains('@') {
            return Err(AuthError::ValidationError("Invalid email address".into()));
        }
        if req.password.len() < 6 {
            return Err(AuthError::ValidationError("Password must be at least 6 characters".into()));
        }

        let password_hash = self.hash_password(&req.password)?;
        Ok(User::new(id, req.username, req.email, password_hash, role))
    }

    /// ตรวจสอบ Credential ของผู้ใช้ และสร้าง AuthResponse
    pub fn authenticate_user(&self, user: &User, password: &str) -> Result<AuthResponse, AuthError> {
        if !user.is_active() {
            return Err(AuthError::AccountInactive);
        }

        let is_valid = self.verify_password(password, &user.password_hash)?;
        if !is_valid {
            return Err(AuthError::InvalidCredentials);
        }

        let token = self.generate_token(user)?;
        Ok(AuthResponse {
            access_token: token,
            token_type: "Bearer".to_string(),
            user: user.to_response(),
        })
    }

    /// ตรวจสอบรหัสผ่านเดิม และคืนค่า password hash ใหม่
    pub fn process_change_password(
        &self,
        user: &User,
        old_password: &str,
        new_password: &str,
    ) -> Result<String, AuthError> {
        let is_valid = self.verify_password(old_password, &user.password_hash)?;
        if !is_valid {
            return Err(AuthError::InvalidCredentials);
        }

        if new_password.len() < 6 {
            return Err(AuthError::ValidationError(
                "New password must be at least 6 characters".into(),
            ));
        }

        self.hash_password(new_password)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_password_hash_and_verify() {
        let auth = AuthService::new(AuthConfig::default());
        let password = "my_secure_password_123";

        let hash = auth.hash_password(password).expect("Hashing should succeed");
        assert_ne!(password, hash);

        let is_valid = auth.verify_password(password, &hash).expect("Verification should succeed");
        assert!(is_valid);

        let is_invalid = auth.verify_password("wrong_password", &hash).expect("Verification should succeed");
        assert!(!is_invalid);
    }

    #[test]
    fn test_jwt_generate_and_verify() {
        let auth = AuthService::new(AuthConfig::default());
        let user = User::new(
            "user_123".into(),
            "trader_bob".into(),
            "bob@example.com".into(),
            "dummy_hash".into(),
            Some(Role::Trader),
        );

        let token = auth.generate_token(&user).expect("Token generation should succeed");
        assert!(!token.is_empty());

        let claims = auth.verify_token(&token).expect("Token verification should succeed");
        assert_eq!(claims.sub, "user_123");
        assert_eq!(claims.username, "trader_bob");
        assert_eq!(claims.role, Role::Trader);
    }
}
