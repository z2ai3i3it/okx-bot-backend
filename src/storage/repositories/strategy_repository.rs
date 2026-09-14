use chrono::Utc;
use futures_util::TryStreamExt;
use mongodb::{
    bson::{doc, to_bson},
    error::Result as MongoResult,
    Collection, Database,
};

use crate::domain::strategy::{Strategy, StrategyConfig, StrategyStatus};

pub struct StrategyRepository {
    collection: Collection<Strategy>,
}

impl StrategyRepository {
    pub fn new(db: &Database) -> Self {
        Self {
            collection: db.collection::<Strategy>("strategies"),
        }
    }

    /// บันทึก Strategy ใหม่ลง MongoDB
    pub async fn create(&self, strategy: &Strategy) -> MongoResult<()> {
        self.collection.insert_one(strategy).await?;
        Ok(())
    }

    /// ค้นหา Strategy ตาม ID
    pub async fn find_by_id(&self, id: &str) -> MongoResult<Option<Strategy>> {
        self.collection.find_one(doc! { "id": id }).await
    }

    /// ค้นหา Strategy ตาม ID และ User ID (สำหรับ Tenant Isolation)
    pub async fn find_by_id_and_user_id(
        &self,
        id: &str,
        user_id: &str,
    ) -> MongoResult<Option<Strategy>> {
        self.collection
            .find_one(doc! { "id": id, "user_id": user_id })
            .await
    }

    /// ดึงรายการ Strategy ทั้งหมดของ User
    pub async fn find_by_user_id(&self, user_id: &str) -> MongoResult<Vec<Strategy>> {
        let cursor = self.collection.find(doc! { "user_id": user_id }).await?;
        let strategies = cursor.try_collect().await?;
        Ok(strategies)
    }

    /// อัปเดตข้อมูล Config และ Name (ทำได้เฉพาะเมื่อไม่ได้รันอยู่)
    pub async fn update_config_and_name(
        &self,
        id: &str,
        user_id: &str,
        name: Option<String>,
        config: Option<StrategyConfig>,
    ) -> MongoResult<bool> {
        let mut update_doc = doc! {
            "updated_at": to_bson(&Utc::now()).unwrap_or_default()
        };

        if let Some(n) = name {
            update_doc.insert("name", n);
        }

        if let Some(c) = config {
            if let Ok(config_bson) = to_bson(&c) {
                update_doc.insert("config", config_bson);
            }
        }

        let filter = doc! { "id": id, "user_id": user_id };
        let update = doc! { "$set": update_doc };

        let result = self.collection.update_one(filter, update).await?;
        Ok(result.matched_count > 0)
    }

    /// อัปเดตสถานะการทำงานของ Strategy (Created -> Running -> Stopped)
    pub async fn update_status(&self, id: &str, status: StrategyStatus) -> MongoResult<bool> {
        let status_bson = to_bson(&status).unwrap_or_default();
        let filter = doc! { "id": id };
        let update = doc! {
            "$set": {
                "status": status_bson,
                "updated_at": to_bson(&Utc::now()).unwrap_or_default()
            }
        };

        let result = self.collection.update_one(filter, update).await?;
        Ok(result.matched_count > 0)
    }

    /// ลบ Strategy ออกจาก MongoDB ตาม ID และ User ID
    pub async fn delete_by_id_and_user_id(&self, id: &str, user_id: &str) -> MongoResult<bool> {
        let filter = doc! { "id": id, "user_id": user_id };
        let result = self.collection.delete_one(filter).await?;
        Ok(result.deleted_count > 0)
    }
}
