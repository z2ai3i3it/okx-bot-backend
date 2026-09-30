# Development Checklist: Bot Strategy & Pipeline Integration

เอกสารนี้ใช้สำหรับติดตามความคืบหน้าการพัฒนาแบบเป็นขั้นเป็นตอน (Step-by-Step) เพื่อไม่ให้สับสนและป้องกันปัญหาบริบทหลุด/Token Limit

---

## 📋 ภาพรวมสถาปัตยกรรม (Architecture Flow)
```text
[Public WS: Ticker] / [Private WS: Order/Fill]
       │
       ▼
[BotExecutor] (Event Loop / Lifecycle)
       │
       ▼
[Strategy Trait (base.rs)] ──(FixedRatioStrategy)──> สร้าง Order Action (+/- 5000 tick)
       │
       ▼ (ส่งคำสั่งเข้า Pipeline)
[Order Pipeline]
   ├── Step 1: Dedup (dedup.rs)
   ├── Step 2: CEM Gatekeeper (cem_step.rs) [Mock/Pass-through]
   ├── Step 3: Rate Limiter (rate_limit.rs)
   └── Step 4: OKX WS Executor (okx_executor.rs / ws_trade.rs)
```

---

## Phase 1: นิยาม Trait และ Types พื้นฐาน (`src/bot/strategies/base.rs`)
- [x] **1.1 สร้าง Trait `Strategy` (หรือ `TradingStrategy`)**
  - [x] Method `on_ticker(&mut self, ticker: &OkxTickerData) -> Vec<StrategyAction>` *(หลีกเลี่ยง log พร่ำเพรื่อกันค้าง)*
  - [x] Method `on_order_update(&mut self, order: &OkxOrderUpdateData) -> Vec<StrategyAction>` *(Real-time Lifecycle: Submit, Partially Filled, Filled, Canceled, Rejected)*
  - [x] Method `on_fill_update(&mut self, fill: &OkxMyTradeData) -> Vec<StrategyAction>` *(ติดตามรายการเทรดจริง/Fill)*
- [x] **1.2 นิยาม Action Enum `StrategyAction`**
  - [x] `PlaceOrder(WsPlaceOrderArg)` (รองรับระบุ bid/ask ฐานสำหรับ audit log)
  - [x] `CancelOrder(WsCancelOrderArg)`
  - [x] `AmendOrder(WsAmendOrderArg)`
  - [x] `Hold` / No-op
- [x] **1.3 เปิด Module ใน `src/bot/strategies/mod.rs` และ `src/bot/mod.rs`**

---

## Phase 2: Mock Strategy สำหรับทดสอบ Flow (`src/bot/strategies/fixed_ratio.rs`)
- [x] **2.1 สร้าง Struct `FixedRatioStrategy`**
  - [x] เก็บ State: ติดตามว่าเคยวาง Order ทดสอบไปหรือยัง (วางแค่ 1 คู่ Buy/Sell)
- [x] **2.2 Implement Trait `Strategy` สำหรับ `FixedRatioStrategy`**
  - [x] เมื่อ Ticker ตัวแรกเข้ามา:
    - ดึงราคา `bid` และ `ask` ล่าสุด
    - คำนวณราคา Buy = `bid - 5000 tick` (หรือ price offset)
    - คำนวณราคา Sell = `ask + 5000 tick`
    - สร้าง `StrategyAction::PlaceOrder` ทั้ง 2 ฝั่ง
    - แสดง log ชัดเจนพร้อมแนบราคา Original Bid/Ask ที่ใช้เป็นฐาน เพื่อยืนยันว่า Ticker มาจริง
- [x] **2.3 จัดการ Event Order / Fill**
  - [x] log อัปเดตสถานะของคำสั่งเมื่อได้รับ `on_order_update` และ `on_fill_update`

---

## Phase 3: Order Pipeline พื้นฐาน (`src/pipeline/`)
- [x] **3.1 กำหนด `PipelineContext` ใน `src/pipeline/context.rs`**
  - [x] เก็บข้อมูล Intent, Client Order ID, Account ID, Strategy ID, Status
- [x] **3.2 สร้าง Pipeline Step: CEM Gatekeeper ใน `src/pipeline/steps/cem_step.rs`**
  - [x] ทำงานแบบ Pass-through / Mock ก่อน (รับมาแล้วส่งต่อเพื่อการทดสอบ flow โครงสร้าง)
- [x] **3.3 สร้าง Pipeline Step: Dedup ใน `src/pipeline/steps/dedup.rs`**
  - [x] ตรวจจับคำสั่งซ้ำด้วย `clOrdId`
- [x] **3.4 สร้าง Pipeline Step: Rate Limiter ใน `src/pipeline/steps/rate_limit.rs`**
  - [x] ขอ permit จาก `OkxRateLimiter`
- [x] **3.5 สร้าง Pipeline Step: OKX Executor ใน `src/pipeline/steps/okx_executor.rs`**
  - [x] ส่งคำสั่งไปยัง `OkxWsTradeClient`
- [x] **3.6 เชื่อมร้อย Pipeline Chain ใน `src/pipeline/order_pipeline.rs`**

---

## Phase 4: เชื่อมต่อเข้ากับ `BotExecutor` (`src/bot/executor.rs`)
- [x] **4.1 เพิ่ม Private WS streams ให้ `BotExecutor`**
  - [x] Subscribe ช่อง `orders` และ `my_trades` จาก `OkxWsTradeClient`
- [x] **4.2 ปรับ Event Loop ใน `run()`**
  - [x] เชื่อม Ticker $\rightarrow$ `strategy.on_ticker()` (ไม่ใส่ log รัวๆ)
  - [x] เชื่อม Order $\rightarrow$ `strategy.on_order_update()`
  - [x] เชื่อม Fill $\rightarrow$ `strategy.on_fill_update()`
  - [x] เมื่อมี `StrategyAction::PlaceOrder / CancelOrder` $\rightarrow$ ส่งเข้า `OrderPipeline`
- [x] **4.3 ตรวจสอบ Graceful Shutdown**

---

## Phase 5: Verification & End-to-End Test (Mock Pipeline)
- [x] **5.1 ตรวจสอบการคอมไพล์และเทสต์ (`cargo test` - 14 passed)**
- [x] **5.2 ยืนยันการไม่ flood log จาก Ticker (Snap แสดงผลเฉพาะตอนวาง Order เท่านั้น)**
  - [x] Ticker stream ทำงานเงียบใน background ไม่พิมพ์ log ทุก tick
  - [x] Snap log แสดงผลเฉพาะจังหวะ Place Order พร้อมแสดง Original Bid/Ask ครบถ้วน
  - [x] วางคำสั่งคู่ Buy/Sell (+/- 5000 tick) สำเร็จเข้าสู่ Pipeline Chain (Dedup -> CEM -> RateLimit -> OKX Executor)

---

## Phase 6: Fixed Ratio Strategy Implementation (`src/bot/strategies/fixed_ratio.rs`)
ถอดบทเรียนและหลอมรวม logic จาก Python (`logs/fixed_ratio.py` - WS/Event-driven/Trade-Centric) และ Rust draft (`logs/fixed_ratio_strategy.rs` - REST/Downgraded) ให้กลายเป็นสถาปัตยกรรม Event-driven บน WebSocket Private + Public เต็มรูปแบบ

- [x] **6.1 Data Structures & Strategy State Management**
  - [x] รับและเก็บ `FixdRatioRebalanceConfig` จาก `StrategyModel`
  - [x] ออกแบบ State ภายใน `FixedRatioStrategy`:
    - `last_snapshot`: Snapshot สถานะ Rebalance ล่าสุด (Net Base Balance, Quote Value, Target Anchors, Sequence)
    - `tracked_buy` / `tracked_sell`: ออเดอร์ที่กำลังเปิดอยู่ (`clOrdId`, `price`, `amount`, `side`, `status`)
    - `cumulative_filled`: สะสม fill size ต่อ clOrdId ป้องกัน race condition ระหว่าง order closed กับ fill events
    - `last_trade_waiters`: ตัวดักรอ trade fill ครบก่อนสั่ง cleanup / start new rebalance
  - [x] ผูกคณิตศาสตร์การคำนวณจาก `src/bot/math/fixed_ratio_cal.rs`:
    - `cal_target_price()`
    - `cal_target_amount_improved()`
    - `save_to_db_first_filled_rebalance()`
    - `save_to_db_filled_rebalance()`

- [x] **6.2 Phase แรก: First Order Flow (`rebalance_first`)**
  - [x] เมื่อ Ticker แรกเข้ามา และยังไม่มี Snapshot:
    - คำนวณ Portfolio Balance เริ่มต้น (`base_value` vs `quote_value`)
    - หา `order_side` (Buy ถ้าถือ quote มากกว่า, Sell ถ้าถือ base มากกว่า)
    - คำนวณ `amount_improved`
  - [x] จัดการกรณี Balanced ตั้งแต่ต้น (`amount == 0` หรือ `< min_size_b`):
    - สร้าง Seed Snapshot ทันที
    - Trigger เข้าสู่ `rebalance_cal` เพื่อวาง Order คู่ Buy & Sell
  - [x] จัดการกรณี Imbalanced:
    - วาง Order เดี่ยว 1 ฝั่งแบบ `post_only` ด้วยราคาห่างตลาด (Adjusted Price)
    - บันทึกสถานะเพื่อรอรับ Fill แล้วค่อยทำ Rebalance รอบถัดไป

- [x] **6.3 Normal Cycle: Rebalance Calculation (`rebalance_cal`)**
  - [x] ตรวจสอบ Balance Availability (`base > 0` และ `quote > 0`)
  - [x] คำนวณราคาและขนาดเป้าหมายพร้อมกันทั้ง 2 ฝั่ง (`new_buy_price`, `new_sell_price`, `amount_buy`, `amount_sell`)
  - [x] กำหนด Target Anchor Order สำหรับฝั่ง Buy และ Sell
  - [x] สร้างและส่ง `StrategyAction::PlaceOrder` (Post-Only) ทั้ง 2 ฝั่งเข้า Order Pipeline

- [x] **6.4 Real-time Event Handling (Order Update & Trade/Fill Events)**
  - [x] **`on_order_update(&mut self, order: &OkxOrderUpdateData)`**:
    - **Filled**: สั่ง Cancel Sibling ทันที (มีได้แค่ 1 ฝั่งที่ fill)
    - **Safety Gate Waiter**: เช็คว่า Trades ครบตาม `filled` หรือยัง หากยังไม่ครบ ให้เข้า Waiter รอ
    - **Canceled / Rejected**: ตรวจสอบว่าเป็น Planned (Sibling Cancel) หรือ Unplanned (โดนยกเลิกผิดปกติ)
    - **Self-Healing**: หากเป็น Unplanned Cancel ให้เยียวยาด้วยการวางคำสั่งใหม่ที่ราคาตลาด
  - [x] **`on_fill_update(&mut self, fill: &OkxMyTradeData)` (Trade-Centric)**:
    - ตรวจสอบ Deduplication trade ID
    - สะสม `cumulative_filled`
    - คำนวณ Fee (Maker vs Taker)
    - อัปเดต Snapshot Rebalance ทันที (`save_to_db_filled_rebalance`)
    - ตรวจสอบ Waiter: หาก Trade ครบแล้ว ให้ Finalize Order และเคลียร์ Memory
  - [x] **Finalize Order**:
    - ล้าง tracked orders
    - เมื่อออเดอร์ทั้งหมดถูกเคลียร์ (Clean Slate) $\rightarrow$ Trigger `rebalance_cal()` รอบถัดไปอัตโนมัติ

- [x] **6.5 Integration & Edge Case Handling**
  - [x] เชื่อม `FixdRatioRebalanceConfig` จาก DB/Strategy Model ใน `BotExecutor::new`
  - [x] รองรับ Graceful Shutdown: สั่ง Cancel ออเดอร์ที่ค้างอยู่ทั้งหมดเมื่อบอทหยุดทำงาน
  - [x] Verification & Testing: Unit tests สำหรับ state transitions (ผ่านครบ 21/21 tests)

---

## 📌 สถานะปัจจุบัน: สิ่งที่ยังไม่ได้นำมา (Pending / Missing Features)

1. **Database Persistence (บันทึก Snapshot จริงลง Database)**
   - ปัจจุบัน `snapshot` ยังคงอยู่ใน In-Memory (`Option<RebalanceRecord>`) เท่านั้น
   - หากโปรเซสบอทดับหรือรีสตาร์ท ค่า sequence, net base balance และ quote value จะหายไป
   - ยังไม่มี `RebalanceRepository` สำหรับ collection `rebalance_records` ใน MongoDB

2. **Recovery Manager (`one_time_execute`)**
   - ยังไม่มีขั้นตอนตรวจสอบ State เดิมก่อนเข้า Event Loop
   - เมื่อบอทสตาร์ทใหม่ ยังไม่ได้ดึง Snapshot ล่าสุดจาก DB
   - ยังไม่ได้ตรวจสอบและ Replay Trades ที่เกิดขึ้นระหว่างที่บอทปิดไป (Replay Fills)
   - ยังไม่ได้สั่ง Clean Slate Cancel ออเดอร์ที่ค้างบน Exchange ก่อนเริ่มรอบใหม่

3. **Chasing Market Price (Optional Feature ใน Python)**
   - ฟีเจอร์ไล่ราคาตลาดตาม Spread / Depth ใน `logs/fixed_ratio.py` ยังไม่ได้นำมาใส่

---

## Phase 7: Database Persistence & Fault-Tolerant Generic Recovery

### 🧠 สถาปัตยกรรม Recovery: Generic vs Strategy-Specific (ป้องกัน Rebalance Bias)
เพื่อไม่ให้ `src/bot/recovery.rs` ผูกติด (Bias) กับ Rebalance กลยุทธ์เดียว และสามารถรองรับกลยุทธ์อื่นๆ เช่น Grid ได้อย่างไร้รอยต่อ จึงแบ่งขอบเขตหน้าที่อย่างชัดเจน:
1. **Generic Recovery Engine (`src/bot/recovery.rs`)**:
   - ทำงานเฉพาะระดับ **Exchange & Order Level** (ไม่แตะต้องสูตรคำนวณพอร์ต)
   - **Clean Slate**: สั่ง Cancel คำสั่งที่ค้างอยู่บน OKX ทั้งหมดของบอทตัวนี้
   - **Fetch Missed Trades**: ตรวจสอบคำสั่งค้างและดึงประวัติ Trade Fills ที่เกิดขึ้นระหว่างบอทออฟไลน์ (ผ่าน REST API)
   - **Delegate**: ส่งรายการ Trades ทั้งหมดที่รวบรวมได้ มอบหมายให้ `Strategy::on_recovery_reconcile(&missed_trades)` เป็นคนจัดการต่อตามตรรกะของตนเอง
2. **Strategy-Specific Replay (`src/bot/strategies/fixed_ratio.rs` & `base.rs`)**:
   - เพิ่ม Hook `on_recovery_reconcile` ใน Trait `Strategy` (Default implementation ว่างเปล่า)
   - `FixedRatioStrategy`: รับ Missed Trades $\rightarrow$ นำมา Replay คำนวณ Rebalance Snapshot ต่อเนื่อง (`save_to_db_filled_rebalance`) $\rightarrow$ Restore State พอร์ตให้พร้อมเริ่มรอบใหม่
   - `GridStrategy` (ในอนาคต): รับ Missed Trades $\rightarrow$ ตรวจจับว่าไม้ไหน Fill เพื่อเตรียม Re-grid
3. **Dedicated Rebalance Persistence (`src/storage/` & `src/services/`)**:
   - แยก `RebalanceRepository` และ `RebalanceService` ออกมาเฉพาะสำหรับงานบันทึก Rebalance History โดยตรง ไม่ปนกับ Core Engine

---

### 📋 แผนงานพัฒนา Phase 7:

- [x] **7.1 Layer 11 Persistence: `RebalanceRepository` (`src/storage/repositories/rebalance_repository.rs`)**
  - [x] ออกแบบ Document Schema สำหรับ collection `rebalance_records` (มี `bot_id`, `sequence`, `price_qb`, `net_base_balance_b`, ฯลฯ)
  - [x] Method `insert_record(&self, record: &Rebalance) -> MongoResult<()>`
  - [x] Method `get_last_record(&self, bot_id: &str) -> MongoResult<Option<Rebalance>>`
  - [x] Method `count_by_bot(&self, bot_id: &str) -> MongoResult<u64>`

- [x] **7.2 Layer 9 Service: `RebalanceService` (`src/services/rebalance_service.rs`)**
  - [x] ทำหน้าที่เป็น Application Service สำหรับ Rebalance Lifecycle
  - [x] Method `save_trade_fill(&self, bot_id: &str, order_id: &str, record: &RebalanceRecord) -> Result<Rebalance, RebalanceServiceError>`
  - [x] Method `get_last_snapshot(&self, bot_id: &str) -> Result<Option<RebalanceRecord>, RebalanceServiceError>`

- [x] **7.3 Generic Recovery Engine (`src/bot/recovery.rs`)**
  - [x] สร้าง Struct `RecoveryEngine` ที่เป็น Generic (ไม่ผูกกับ Rebalance)
  - [x] Method `run_recovery(bot_id, inst_id, pending_client_order_ids, trade_client, missed_trades, strategy: &mut dyn Strategy) -> Result<(), RecoveryError>`:
    - **Step 1: Clean Slate Cancel**: สั่ง Cancel คำสั่งที่ยังค้างอยู่บน OKX ทั้งหมดของบอทตัวนี้
    - **Step 2: Fetch Missed Fills**: ตรวจสอบคำสั่งค้างและประวัติการ Match (Trades)
    - **Step 3: Delegate to Strategy**: เรียก `strategy.on_recovery_reconcile(&missed_trades)` เพื่อให้กลยุทธ์ Replay ตัวเอง

- [x] **7.4 Strategy Hook & Fixed Ratio Replay (`src/bot/strategies/base.rs` & `fixed_ratio.rs`)**
  - [x] เพิ่ม `fn on_recovery_reconcile(&mut self, missed_trades: &[OkxMyTradeData]) -> Result<(), String>` ใน Trait `Strategy`
  - [x] Implement ใน `FixedRatioStrategy`:
    - นำ Trades ที่ได้มา Sort ตาม Timestamp (เก่า $\rightarrow$ ใหม่)
    - Replay ทีละ Trade ผ่านสูตรคำนวณ `save_to_db_filled_rebalance`
    - ตั้งค่า `self.snapshot` ล่าสุดให้พร้อมสำหรับการคำนวณ Rebalance รอบแรกทันทีที่เข้า Event Loop

- [x] **7.5 Layer 7 Integration: เชื่อมต่อเข้ากับ `BotExecutor` (`src/bot/executor.rs`)**
  - [x] ใน `BotExecutor::run()`:
    - เรียก `RecoveryEngine::run_recovery()` ให้เรียบร้อยก่อนเปิดรับ WebSocket Event Loop
    - เมื่อมี Event `on_fill_update` ใน Strategy $\rightarrow$ ส่ง Record บันทึกลง Database ผ่าน `RebalanceService` แบบ Non-blocking Async Task
  - [x] Unit Test & Verification: ทดสอบ Recovery Flow จำลองทั้งกรณีไม่มีออเดอร์ค้าง และกรณีมี Missed Trades ตกค้าง