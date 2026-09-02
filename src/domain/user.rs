
use crate::domain::account::Account;

pub struct User {
    pub id: String,
    pub name: String,
    pub email: String,
    pub accounts: Vec<Account>,
    // pub role: Role,
    // pub permissions: Vec<Permission>,
    // อาจจะเพิ่มเติมฟิลด์ได้ตามความต้องการ
}
impl User {
    pub fn new(id: String, name: String, email: String, accounts: Vec<Account>) -> Self {
        Self {
            id,
            name,
            email,
            accounts,
        }
    }
}
