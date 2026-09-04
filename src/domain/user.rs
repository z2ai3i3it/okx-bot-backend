use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// สิทธิ์การใช้งานระบบของผู้ใช้ (Role-Based Access Control)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Trader,
    Viewer,
}

impl Default for Role {
    fn default() -> Self {
        Role::Trader
    }
}

/// สถานะของบัญชีผู้ใช้
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum UserStatus {
    Active,
    Suspended,
    PendingVerification,
}

impl Default for UserStatus {
    fn default() -> Self {
        UserStatus::Active
    }
}

/// ข้อมูลหลักของ User Entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    pub email: String,
    
    /// Password hash (เช่น Argon2 หรือ bcrypt)
    pub password_hash: String,
    
    pub role: Role,
    pub status: UserStatus,
    
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
}

impl User {
    /// สร้าง User ใหม่สำหรับตอนสมัครสมาชิก
    pub fn new(
        id: String,
        username: String,
        email: String,
        password_hash: String,
        role: Option<Role>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id,
            username,
            email,
            password_hash,
            role: role.unwrap_or_default(),
            status: UserStatus::Active,
            created_at: now,
            updated_at: now,
            last_login_at: None,
        }
    }

    /// ตรวจสอบว่าผู้ใช้มีสิทธิ์ระดับ Admin หรือไม่
    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
    }

    /// ตรวจสอบว่าบัญชีอยู่ในสถานะเปิดใช้งานหรือไม่
    pub fn is_active(&self) -> bool {
        self.status == UserStatus::Active
    }

    /// แปลงเป็น Safe User Response สำหรับส่งกลับ Frontend (ไม่เปิดเผย password_hash)
    pub fn to_response(&self) -> UserResponse {
        UserResponse {
            id: self.id.clone(),
            username: self.username.clone(),
            email: self.email.clone(),
            role: self.role.clone(),
            status: self.status.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            last_login_at: self.last_login_at,
        }
    }
}

/// JWT Claims สำหรับการสร้างและถอดรหัส Token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,       // User ID
    pub username: String,  // Username
    pub role: Role,        // Role สำหรับเช็ค Authorization
    pub exp: usize,        // Unix timestamp เวลาหมดอายุ
    pub iat: usize,        // Unix timestamp เวลาที่สร้าง
}

/// Data Transfer Object สำหรับส่งข้อมูล User กลับไปยัง Frontend
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UserResponse {
    pub id: String,
    pub username: String,
    pub email: String,
    pub role: Role,
    pub status: UserStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
}

/// Payload ข้อมูลสำหรับการสมัครสมาชิกใหม่
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RegisterRequest {
    #[schema(example = "trader_alice")]
    pub username: String,
    #[schema(example = "alice@example.com")]
    pub email: String,
    #[schema(example = "Password123!")]
    pub password: String,
}

/// Payload ข้อมูลสำหรับการเข้าสู่ระบบ
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LoginRequest {
    #[schema(example = "alice@example.com")]
    pub email: String,
    #[schema(example = "Password123!")]
    pub password: String,
}

/// Response ที่ส่งกลับเมื่อเข้าสู่ระบบสำเร็จ
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AuthResponse {
    #[schema(example = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...")]
    pub access_token: String,
    #[schema(example = "Bearer")]
    pub token_type: String,
    pub user: UserResponse,
}

/// Response format ทั่วไปสำหรับ Success
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GenericMessageResponse {
    #[schema(example = true)]
    pub success: bool,
    #[schema(example = "Operation completed successfully")]
    pub message: String,
}

/// Payload สำหรับอัปเดตข้อมูลส่วนตัวของผู้ใช้ (Update Profile)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateProfileRequest {
    #[schema(example = "alice_pro")]
    pub username: Option<String>,
    #[schema(example = "alice_new@example.com")]
    pub email: Option<String>,
}

/// Payload สำหรับเปลี่ยนรหัสผ่าน (Change Password)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChangePasswordRequest {
    #[schema(example = "Password123!")]
    pub old_password: String,
    #[schema(example = "NewPassword456!")]
    pub new_password: String,
}


