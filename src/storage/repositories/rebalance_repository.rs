use mongodb::{
    Collection, Database, bson::doc, error::Result as MongoResult, options::FindOneOptions,
};

use crate::domain::rebalance::Rebalance;

/// Repository สำหรับจัดการบันทึกและอ่านข้อมูล Rebalance Record จาก MongoDB collection `rebalance_records`
pub struct RebalanceRepository {
    collection: Collection<Rebalance>,
}

impl RebalanceRepository {
    pub fn new(db: &Database) -> Self {
        Self {
            collection: db.collection::<Rebalance>("rebalances"),
        }
    }

    /// บันทึก Rebalance record ใหม่
    pub async fn insert_record(&self, record: &Rebalance) -> MongoResult<()> {
        self.collection.insert_one(record).await?;
        Ok(())
    }

    /// ค้นหา Rebalance record ล่าสุดของบอทตัวนี้ (Sort ตาม sequence ถอยหลัง)
    pub async fn get_last_record(&self, bot_id: &str) -> MongoResult<Option<Rebalance>> {
        let filter = doc! { "bot_id": bot_id };
        let options = FindOneOptions::builder()
            .sort(doc! { "sequence": -1 })
            .build();

        self.collection.find_one(filter).with_options(options).await
    }

    /// นับจำนวน Rebalance record ทั้งหมดของบอทตัวนี้
    pub async fn count_by_bot(&self, bot_id: &str) -> MongoResult<u64> {
        let filter = doc! { "bot_id": bot_id };
        self.collection.count_documents(filter).await
    }
}

#[cfg(test)]
mod tests {
    use super::RebalanceRepository;

    #[test]
    fn test_rebalance_repository_interface() {
        // ยืนยันว่า struct สามารถเข้ากันได้กับการคอมไพล์
        let _ = std::mem::size_of::<RebalanceRepository>();
    }
}
