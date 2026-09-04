use futures_util::stream::TryStreamExt;
use mongodb::{
    bson::doc,
    error::Result as MongoResult,
    Collection, Database,
};
use crate::domain::account::Account;

pub struct AccountRepository {
    collection: Collection<Account>,
}

impl AccountRepository {
    pub fn new(db: &Database) -> Self {
        Self {
            collection: db.collection::<Account>("accounts"),
        }
    }

    /// บันทึก Account ใหม่ลง Collection `accounts`
    pub async fn create(&self, account: &Account) -> MongoResult<()> {
        self.collection.insert_one(account).await?;
        Ok(())
    }

    /// ค้นหา Account ด้วย Account ID
    pub async fn find_by_id(&self, id: &str) -> MongoResult<Option<Account>> {
        self.collection.find_one(doc! { "_id": id }).await
    }

    /// ค้นหารายการ Accounts ทั้งหมดของผู้ใช้ `user_id`
    pub async fn find_by_user_id(&self, user_id: &str) -> MongoResult<Vec<Account>> {
        let mut cursor = self
            .collection
            .find(doc! { "user_id": user_id })
            .await?;

        let mut accounts = Vec::new();
        while let Some(account) = cursor.try_next().await? {
            accounts.push(account);
        }
        Ok(accounts)
    }

    /// ค้นหา Account ของ User โดยเฉพาะ เพื่อเช็คความเป็นเจ้าของ
    pub async fn find_by_id_and_user_id(
        &self,
        id: &str,
        user_id: &str,
    ) -> MongoResult<Option<Account>> {
        self.collection
            .find_one(doc! { "_id": id, "user_id": user_id })
            .await
    }

    /// ลบ Account ออกจาก Database โดยต้องเป็นของ user_id นั้น
    pub async fn delete_by_id_and_user_id(&self, id: &str, user_id: &str) -> MongoResult<bool> {
        let result = self
            .collection
            .delete_one(doc! { "_id": id, "user_id": user_id })
            .await?;
        Ok(result.deleted_count > 0)
    }
}
