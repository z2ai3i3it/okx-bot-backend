use mongodb::{
    bson::{doc, to_document},
    Collection, Database,
};
use thiserror::Error;

use crate::domain::user::User;

#[derive(Debug, Error)]
pub enum UserRepositoryError {
    #[error("MongoDB operation error: {0}")]
    MongoError(#[from] mongodb::error::Error),

    #[error("Serialization error: {0}")]
    BsonError(#[from] mongodb::bson::ser::Error),

    #[error("Deserialization error: {0}")]
    BsonDeError(#[from] mongodb::bson::de::Error),
}

/// Repository สำหรับจัดการเอกสารใน Collection `users`
#[derive(Debug, Clone)]
pub struct UserRepository {
    collection: Collection<User>,
}

impl UserRepository {
    pub const COLLECTION_NAME: &'static str = "users";

    pub fn new(db: &Database) -> Self {
        Self {
            collection: db.collection::<User>(Self::COLLECTION_NAME),
        }
    }

    /// บันทึกผู้ใช้ใหม่ลง MongoDB
    pub async fn create(&self, user: &User) -> Result<(), UserRepositoryError> {
        self.collection.insert_one(user).await?;
        Ok(())
    }

    /// ค้นหาผู้ใช้ด้วย User ID
    pub async fn find_by_id(&self, id: &str) -> Result<Option<User>, UserRepositoryError> {
        let filter = doc! { "id": id };
        let user = self.collection.find_one(filter).await?;
        Ok(user)
    }

    /// ค้นหาผู้ใช้ด้วย Email
    pub async fn find_by_email(&self, email: &str) -> Result<Option<User>, UserRepositoryError> {
        let filter = doc! { "email": email };
        let user = self.collection.find_one(filter).await?;
        Ok(user)
    }

    /// ค้นหาผู้ใช้ด้วย Username
    pub async fn find_by_username(&self, username: &str) -> Result<Option<User>, UserRepositoryError> {
        let filter = doc! { "username": username };
        let user = self.collection.find_one(filter).await?;
        Ok(user)
    }

    /// อัปเดตข้อมูลผู้ใช้
    pub async fn update(&self, user: &User) -> Result<bool, UserRepositoryError> {
        let filter = doc! { "id": &user.id };
        let doc = to_document(user)?;
        let update = doc! { "$set": doc };
        let result = self.collection.update_one(filter, update).await?;
        Ok(result.matched_count > 0)
    }

    /// ลบผู้ใช้ตาม ID
    pub async fn delete_by_id(&self, id: &str) -> Result<bool, UserRepositoryError> {
        let filter = doc! { "id": id };
        let result = self.collection.delete_one(filter).await?;
        Ok(result.deleted_count > 0)
    }
}
