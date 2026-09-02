use serde::{Deserialize, Serialize};
use crate::domain::user::{Claims, Role};

/// สิทธิ์การเข้าถึงทรัพยากรระดับฟังก์ชันการทำงาน (Granular Permissions)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    // การจัดการผู้ใช้
    ManageUsers,
    ViewUsers,

    // การจัดการ OKX API Key
    LinkExchangeAccount,
    ViewExchangeAccount,
    DeleteExchangeAccount,

    // การควบคุมบอทเทรด
    StartBot,
    StopBot,
    PauseBot,
    ConfigureStrategy,

    // ข้อมูลสถิติและการเงิน
    ViewPnL,
    ViewTradeHistory,
    ViewSystemMetrics,
}

/// Helper struct สำหรับประเมินสิทธิ์ (Authorization Policy)
pub struct PermissionGuard;

impl PermissionGuard {
    /// ดึงรายการสิทธิ์ทั้งหมดที่ผูกกับ Role นั้นๆ
    pub fn get_permissions_by_role(role: &Role) -> Vec<Permission> {
        match role {
            Role::Admin => vec![
                Permission::ManageUsers,
                Permission::ViewUsers,
                Permission::LinkExchangeAccount,
                Permission::ViewExchangeAccount,
                Permission::DeleteExchangeAccount,
                Permission::StartBot,
                Permission::StopBot,
                Permission::PauseBot,
                Permission::ConfigureStrategy,
                Permission::ViewPnL,
                Permission::ViewTradeHistory,
                Permission::ViewSystemMetrics,
            ],
            Role::Trader => vec![
                Permission::LinkExchangeAccount,
                Permission::ViewExchangeAccount,
                Permission::DeleteExchangeAccount,
                Permission::StartBot,
                Permission::StopBot,
                Permission::PauseBot,
                Permission::ConfigureStrategy,
                Permission::ViewPnL,
                Permission::ViewTradeHistory,
            ],
            Role::Viewer => vec![
                Permission::ViewExchangeAccount,
                Permission::ViewPnL,
                Permission::ViewTradeHistory,
            ],
        }
    }

    /// ตรวจสอบว่า Role ปัจจุบันมีสิทธิ์ตามที่ระบุหรือไม่
    pub fn has_permission(role: &Role, permission: &Permission) -> bool {
        let permissions = Self::get_permissions_by_role(role);
        permissions.contains(permission)
    }

    /// ตรวจสอบว่า Claims ปัจจุบันมีสิทธิ์การเข้าถึงหรือไม่
    pub fn can(claims: &Claims, permission: &Permission) -> bool {
        Self::has_permission(&claims.role, permission)
    }

    /// ตรวจสอบความเป็นเจ้าของทรัพยากร (Resource Ownership Check)
    /// Admin เข้าถึงได้เสมอ ส่วน Trader จะเข้าถึงได้เฉพาะข้อมูลของตนเอง
    pub fn can_access_resource(claims: &Claims, resource_owner_id: &str) -> bool {
        if claims.role == Role::Admin {
            return true;
        }
        claims.sub == resource_owner_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_admin_has_all_permissions() {
        let admin_role = Role::Admin;
        assert!(PermissionGuard::has_permission(&admin_role, &Permission::ManageUsers));
        assert!(PermissionGuard::has_permission(&admin_role, &Permission::StartBot));
        assert!(PermissionGuard::has_permission(&admin_role, &Permission::ViewSystemMetrics));
    }

    #[test]
    fn test_trader_permissions() {
        let trader_role = Role::Trader;
        assert!(!PermissionGuard::has_permission(&trader_role, &Permission::ManageUsers));
        assert!(!PermissionGuard::has_permission(&trader_role, &Permission::ViewSystemMetrics));
        assert!(PermissionGuard::has_permission(&trader_role, &Permission::StartBot));
        assert!(PermissionGuard::has_permission(&trader_role, &Permission::ViewPnL));
    }

    #[test]
    fn test_viewer_cannot_start_bot() {
        let viewer_role = Role::Viewer;
        assert!(!PermissionGuard::has_permission(&viewer_role, &Permission::StartBot));
        assert!(PermissionGuard::has_permission(&viewer_role, &Permission::ViewPnL));
    }

    #[test]
    fn test_resource_ownership() {
        let trader_claims = Claims {
            sub: "user_bob".into(),
            username: "bob".into(),
            role: Role::Trader,
            exp: 100000,
            iat: 90000,
        };

        let admin_claims = Claims {
            sub: "user_admin".into(),
            username: "admin".into(),
            role: Role::Admin,
            exp: 100000,
            iat: 90000,
        };

        // Trader เข้าถึงของตัวเองได้ แต่เข้าถึงของคนอื่นไม่ได้
        assert!(PermissionGuard::can_access_resource(&trader_claims, "user_bob"));
        assert!(!PermissionGuard::can_access_resource(&trader_claims, "user_alice"));

        // Admin เข้าถึงทรัพยากรของใครก็ได้
        assert!(PermissionGuard::can_access_resource(&admin_claims, "user_bob"));
        assert!(PermissionGuard::can_access_resource(&admin_claims, "user_alice"));
    }
}
