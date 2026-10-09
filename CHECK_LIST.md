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
   - **สถาปัตยกรรมและ Logic การทำงานของ Chasing:**
     1. **เป้าหมายและหัวใจสำคัญ (Why Chasing?):**
        - ปรับพอร์ตให้ Match สำเร็จตามเป้าหมายโดย **ไม่ใช้ Market Order (Taker)** เพื่อหลีกเลี่ยง Fee แพงและ Slippage
        - ป้องกันออเดอร์ค้าง (Stuck Limit Order) เมื่อราคาตลาดวิ่งหนี ด้วยการส่งคำสั่งเป็น Maker ล่วงหน้า แล้วใช้ Background Task ไล่ขยับราคา (`amend_order`) เกาะหัวแถวของ Orderbook ไปเรื่อยๆ จนกว่าจะ Fill ครบ
     2. **วงจรการทำงาน (Lifecycle & Flow):**
        - **Initial Order Placement (`_create_order_intent_at_market`):** วางคำสั่งแรกโดยเว้นระยะห่างปลอดภัย (Offset 100 ticks หรือ TR) ฝั่ง Buy ต่ำกว่าตลาด / ฝั่ง Sell สูงกว่าตลาด เพื่อเป็น Buffer ป้องกันการ Match ทันทีเป็น Taker
        - **Chasing Loop (`_chasing_order_loop`):** วนลูปตรวจสอบ Best Bid / Best Ask ใน Orderbook:
          - *กรณี Spread กว้าง ($\ge 3$ ticks):* แทรกกลาง Spread (Inside Spread) โดย Buy ตั้ง `bid1 + 1 tick`, Sell ตั้ง `ask1 - 1 tick` เพื่อขึ้นมาเป็น Best Price คิวแรก
          - *กรณี Spread แคบ ($< 3$ ticks):* วางเกาะชิด Best Bid/Ask พร้อมเว้น Safety Margin ไม่ให้ข้ามฝั่งไปชนราคาตลาด
          - *การปรับราคา:* ส่ง `amend_order` อัปเดตราคาใหม่ และจบ Loop ทันทีเมื่อออเดอร์ `filled`, `canceled` หรือเหลือปริมาณ 0
     3. **เงื่อนไขที่เรียกใช้ Chasing (Trigger Cases):**
        - **First Rebalance:** เริ่มต้นรันบอทครั้งแรกแล้วพอร์ตไม่อยู่ในจุดสมดุล (Imbalance เกิน Threshold) ต้องบังคับปรับพอร์ตให้เข้าที่ก่อน
        - **Self-Healing / Incomplete Fill:** มีออเดอร์หลุด ค้าง หรือการ Rebalance ขาดช่วง บอทสร้างออเดอร์ซ่อมแซมเพื่อดึงพอร์ตกลับมาสมดุล
        - **Unplanned Cancel Healing:** คำสั่งถูกยกเลิกผิดปกติจาก Exchange ระหว่างรอบ Rebalance
        *(หมายเหตุ: คำสั่ง Rebalance ปกติแบบ Grid / Ladder จะวางแช่รอ Fill ตามระดับราคา ไม่เข้า Chasing)*

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

---

## Phase 8: Chasing Market Price (Event-driven via Ticker/Orderbook)

- [x] **8.1 สร้าง Action และ State Engine สำหรับ Chasing (Pure Strategy State Machine)**
  - [x] ตรวจสอบ `StrategyAction::AmendOrder(WsAmendOrderArg)` และการ dispatch เข้า Pipeline / WebSocket
  - [x] ออกแบบ `ChasingState` ภายใน `FixedRatioStrategy` ติดตาม `client_order_id`, `side`, `current_price`, `anchor_price`, `last_amend_time`
  - [x] Implement ตรรกะ `calculate_chasing_price` (Spread $\ge 3$ ticks vs $< 3$ ticks พร้อม Anchor Clamping)
  - [x] Throttle ป้องกันการยิง amend order ถี่เกิน 500ms

- [x] **8.2 เรียกใช้ Chasing ใน Case ที่ 1: First Rebalance (`on_ticker`)**
  - [x] ใน `on_ticker`: หากเกิด Imbalance วาง Initial Order หลบราคาตลาด (Buffer 100 ticks) และเซ็ต `chasing_state`
  - [x] ใน Ticker ถัดๆ ไป: หากมี `chasing_state` ให้คำนวณราคาใหม่และ emit `StrategyAction::AmendOrder`
  - [x] ใน `try_finalize_order` และ `on_order_update`: เมื่อออเดอร์มีสถานะ `filled`, `canceled` เคลียร์ `chasing_state = None`
  - [x] Unit Test `test_chasing_order_loop_amends_price_and_finalizes` ผ่านเรียบร้อย (35 passed)

- [x] **8.3 Self-Healing: Order Placement Rejected / Post-Only Rejection**

  **ที่มา (Legacy Ref):**
  - Python `on_order_placement_failed` (L708–L791): แยก error_type เป็น `InvalidOrder`, `OrderImmediatelyFillable`, `OrderNotFillable`, `InsufficientFunds`, `NetworkError`
  - Python `_on_order_rejected` (L1146–L1165): อัปเดต tracked status เป็น REJECTED
  - Rust `send_order_with_healing` (L714–L793): ถ้าวาง post_only ล้มเหลว → วางใหม่ผ่าน `create_order_at_market`

  **Edge Cases ที่ต้องจัดการใน `on_order_update`:**

  - [x] **8.3.1 Post-Only Rejection (OKX sCode ≠ 0)**
    - Trigger: OKX reject เพราะ post_only order จะ cross ทันที (OrderImmediatelyFillable)
    - Action: สร้าง order ใหม่ผ่าน `create_order_intent_at_market` (วางหลบ 100 ticks) → ผูกเข้า `ChasingState` ใหม่
    - Python Ref: `on_order_placement_failed` case `OrderImmediatelyFillable` / `InvalidOrder` / `OrderNotFillable`
    - ⚠️ ได้ `side`, `amount`, `anchor_price` จาก tracked order เดิม / snapshot เรียบร้อย

  - [x] **8.3.2 Insufficient Funds (OKX sCode 51010/51015 / cancelSource 9, 44)**
    - Trigger: ยอดเงินไม่พอสำหรับฝั่งนี้ (เช่น ฝั่ง sibling ยังไม่ fill คืน capital)
    - Action: **ไม่ heal** — เคลียร์ tracked ฝั่งนี้ ปล่อยให้ rebalance รอบถัดไปลองใหม่
    - Python Ref: `on_order_placement_failed` case `InsufficientFunds` → PARKED
    - Rust Ref: `send_order_with_healing` → `Ok(None)`

  - [x] **8.3.3 Rate Limit (OKX 50011 / HTTP 429)**
    - Trigger: ยิง API ถี่เกิน OKX rate limit
    - Action: **ไม่ heal** ทันที — ตรวจสอบ sCode/cancelSource ไม่เข้าเงื่อนไข heal และป้องกันการ re-place ถี่เกิน
    - Python Ref: `on_order_placement_failed` case `NetworkError`
    - Rust Ref: `send_order_with_healing` → `Err` + `is_rate_limit_error`

  - [x] **8.3.4 Network / Timeout Error**
    - Trigger: response หาย (timeout, DNS, TLS error) → ไม่รู้ว่า order เกิดบน exchange หรือไม่
    - Action: **เก็บ tracked ไว้** ไม่ล้าง → WebSocket reconnect / recovery engine จะ poll สถานะจาก exchange
      - ถ้า order มีจริง → adopt (chasing ต่อ)
      - ถ้า order ไม่มี (51603) → recovery engine จัดการ clean slate
    - Python Ref: `on_order_placement_failed` case `NetworkError` / `RequestTimeout`
    - Rust Ref: rebalance_first/rebalance_cal `Err(e)` → เก็บ tracked + back off

  - [x] **8.3.5 Amend Order ล้มเหลวระหว่าง Chasing**
    - Trigger: `AmendOrder` ถูก reject (เช่น order filled ระหว่าง amend, หรือ rate limit)
    - Action: เช็คสถานะ —
      - ถ้า order หายไปแล้ว (filled/canceled) → `on_order_update` เคลียร์ `chasing_state` อัตโนมัติ
      - ถ้า transient → เก็บ `chasing_state` ไว้ ลองคำนวณและ amend ใหม่ในรอบ ticker ถัดไป (มี throttle 500ms guard)
    - Python Ref: `_chasing_market_price` L598-L612 catch `ValueError` → break/continue

  - [x] **8.3.6 Unit Test สำหรับ Self-Healing Cases**
    - `test_self_healing_post_only_rejection_replaces_at_market` ผ่านเรียบร้อย
    - `test_insufficient_funds_does_not_heal_and_finalizes` ผ่านเรียบร้อย (37 tests passed)

- [x] **8.4 Unplanned Cancel Healing: Order ถูก Cancel โดย Exchange/ระบบ**

  **ที่มา (Legacy Ref):**
  - Python `_on_order_canceled` (L1060–L1145): แยก Planned vs Unplanned cancel ผ่าน `cancelSource` + `intent_status`
  - Python `_on_rqm_stale_droped` (L881–L951): จัดการ RQM queue drop (placement / cancel type)
  - Python `_on_rqm_stale_droped_v3` (L955–L1057): version ใหม่ที่แยก self-healing ชัดเจนกว่า
  - Rust `check_and_handle_order_fill_v3` (L1379–L1539): แยก zero-fill cancel vs partial-fill cancel

  **วิธีแยก Planned vs Unplanned Cancel:**

  | สถานการณ์ | ประเภท | ตัวบ่งชี้ | Action |
  |-----------|--------|----------|--------|
  | Sibling ถูก cancel เพราะอีกฝั่ง filled | **Planned** | `planned_cancel_order_ids.contains(clOrdId)` | ❌ ไม่ heal — finalize ปกติ |
  | User กด cancel เองจากภายนอก | **Planned / External** | `cancelSource == "1"` | ❌ ไม่ heal |
  | Strategy สั่ง cancel ภายใน | **Planned** | `planned_cancel_order_ids.contains(clOrdId)` | ❌ ไม่ heal — finalize |
  | OKX system cancel (zero fill) | **Unplanned** | `cancelSource == "0" / "31"` && `fill_sz == 0` | ✅ Heal → วางใหม่ |
  | Partial fill cancel | **Unplanned** | `fill_sz > 0` && `cancel` | ⏳ รอ trade fills + cancel sibling |

  **Edge Cases ที่จัดการแล้วใน `on_order_update`:**

  - [x] **8.4.1 Unplanned Cancel — Zero Fill (ไม่มี partial fill)**
    - Trigger: order ถูก cancel โดยไม่มี fill เลย (cancelSource = "0", "31", etc.)
    - Action: วาง order ใหม่ด้วย `create_order_intent_at_market` (หลบ 100 ticks) → ผูก `ChasingState` ใหม่
    - Balance ไม่เปลี่ยน → ไม่ต้อง rebalance ใหม่ แค่ heal ฝั่งเดิม

  - [x] **8.4.2 Unplanned Cancel — มี Partial Fill ค้าง**
    - Trigger: order ถูก cancel แต่มี fill บางส่วนแล้ว (acc_fill_sz > 0)
    - Action:
      1. นำลง `last_trade_waiters` เพื่อรอ Trade Fills มาครบ (balance จะถูกอัปเดตใน `on_fill_update`)
      2. สั่ง Cancel sibling order ทันที เพราะ balance เปลี่ยนแล้ว
      3. เมื่อ trade fills ครบ `try_finalize_order` จะ trigger rebalance ใหม่รอบถัดไป

  - [x] **8.4.3 Planned Cancel — Sibling Cleanup (อีกฝั่ง filled)**
    - Trigger: order ฝั่ง buy filled → strategy สั่ง cancel order ฝั่ง sell (หรือกลับกัน)
    - Action: บันทึกเข้า `planned_cancel_order_ids` → เมื่อ cancel event กลับมา **ไม่ heal** → finalize ตามปกติ

  - [x] **8.4.4 Cancel ระหว่าง Chasing Loop**
    - Trigger: order ถูก cancel ขณะที่ `ChasingState` ยังทำงานอยู่
    - Action:
      - ถ้า Planned (sibling cleanup): เคลียร์ `chasing_state = None` → finalize ปกติ
      - ถ้า Unplanned: เคลียร์ `chasing_state` เดิม → วาง order ใหม่ → ตั้ง `ChasingState` ใหม่

  - [x] **8.4.5 OKX cancelSource Mapping**
    - ใช้ field `cancelSource` จาก OKX Order WS event เพื่อแยก Planned vs Unplanned:
      - `"0"` = order cancel by system → **Unplanned** (Heal ถ้า zero fill)
      - `"1"` = user cancel → ตรวจสอบ `planned_cancel_order_ids` ถ้าใช่ planned ภายใน หรือ external user cancel → **ไม่ heal**
    - 📄 รายละเอียดค่า cancelSource ทั้งหมดและการจัดกลุ่มดูที่ [OKX_CANCEL_SOURCE.md](./OKX_CANCEL_SOURCE.md)

  - [x] **8.4.6 Unit Test สำหรับ Unplanned Cancel Healing Cases**
    - `test_unplanned_cancel_zero_fill_heals_order_at_market` ผ่านเรียบร้อย
    - `test_planned_cancel_sibling_cleanup_does_not_heal` ผ่านเรียบร้อย
    - `test_unplanned_cancel_with_partial_fill_waits_and_cancels_sibling` ผ่านเรียบร้อย
    - `test_external_user_cancel_source_1_finalizes_without_heal` ผ่านเรียบร้อย (41 tests passed)

- [ ] **9. CEM Gatekeeper (Capital Efficiency Manager)**
  - **Context:** 1 account → หลาย bot → หลาย pair → ต้อง share capital ข้าม bot
  - **Legacy Ref:** [capital_efficiency_manager.py](./logs/capital_efficiency_manager.py), [cem_simulate_cancel_plan.py](./logs/cem_simulate_cancel_plan.py)
  - **Analysis:** [cem_analysis.md](CEM Analysis Artifact)

  - [ ] **9.1 Balance Snapshot + Virtual Lock**
    - [ ] สร้าง `BalanceManager` struct: `HashMap<String, BalanceEntry>` (free/used/total per asset)
    - [ ] Subscribe OKX WS `account` channel เพื่อรับ balance update real-time
    - [ ] Virtual Lock: เมื่อ approve intent → หัก `free`, เพิ่ม `used`
    - [ ] Virtual Unlock: เมื่อ cancel/fill/reject → คืน `used` กลับ `free`
    - [ ] Re-apply locks เมื่อ WS balance update มาใหม่ (ป้องกัน ground-truth override virtual state)
    - Legacy ref: `_check_and_approve_fast()` L625-665, `_release_capital()` L338-364, `_process_balance_update()` L413-443

  - [ ] **9.2 CEM Gate — Approve / Reject**
    - [ ] แปลง `CemStep` จาก pass-through เป็น real gate
    - [ ] Logic: `if balance.free >= estimated_cost → approve, else → trigger cancel plan or park`
    - [ ] CEM ต้องเป็น shared state ข้าม bot (Arc<Mutex<CEM>>) เพราะ 1 account ใช้ร่วมกัน
    - [ ] PlaceOrder: ต้อง estimate cost (amount × price สำหรับ buy, amount สำหรับ sell)
    - [ ] AmendOrder: คำนวณ cost diff (ถ้าราคาเปลี่ยน cost อาจเพิ่ม/ลด)
    - [ ] CancelOrder: ปล่อยผ่านเสมอ (cancel ไม่ใช้ capital เพิ่ม)
    - Legacy ref: `_process_order_intent()` L370-382

  - [ ] **9.3 Cancel Plan (simulate_cancel_plan)**
    - [ ] Port `simulate_cancel_plan()` เป็น Rust pure function
    - [ ] Price Conversion: แปลงราคาทุก order เป็น BTC/USDT scale เพื่อเปรียบเทียบข้าม pair
      - Case 1: `BTC/USDT` → ไม่แปลง
      - Case 2: `XXX/BTC` → `mid_xxxusdt / order_price`
      - Case 3: `XXX/USDT` → `(order_price / mid_xxxusdt) * mid_btcusdt`
      - Case 4: `XXX/YYY` → via USDT intermediary
    - [ ] Distance Score: `abs(converted_price - mid_btcusdt)` → ไกล = cancel ก่อน, ใกล้ = เก็บไว้
    - [ ] Algorithm: cumulative cancel จาก furthest จนกว่า `freed >= deficit`
    - [ ] Smart Parking: ถ้า `freed > deficit` → ใช้ surplus เก็บ order ที่ใกล้ mid ที่สุดไว้
    - [ ] Fallback: ถ้าไม่มี mid price → ใช้ simple priority-based plan
    - Legacy ref: `simulate_cancel_plan()` ทั้งไฟล์, `_simulate_and_handle_plan()` L667-759

  - [ ] **9.4 Tracked Orders Pool**
    - [ ] `HashMap<String, TrackedOrder>` — เก็บ intent ทุกตัวที่ CEM รับรู้
    - [ ] IntentStatus: `Approved → Submitted → Open → Closed/Canceled/Rejected` + `Parked` + `CancelPending`
    - [ ] Heartbeat cleanup: ลบ terminal states หลัง cooldown (60s)
    - Legacy ref: `tracked_orders` dict, `_process_exchange_event()` L445-513

  - [ ] **9.5 Parked Orders & Reevaluation**
    - [ ] เมื่อ intent ถูก park → เก็บไว้ใน pool พร้อม `cem_priority` (distance score)
    - [ ] เมื่อ balance เปลี่ยน (cancel สำเร็จ, fill สำเร็จ) → trigger `reevaluate_parked_orders()`
    - [ ] Sort parked orders by `(cem_priority, -created_at)` → reactivate ตัวที่ใกล้ mid สุดก่อน
    - [ ] เมื่อ reactivate: re-lock capital + emit approval
    - Legacy ref: `_reevaluate_parked_orders()` L837-901

  - [ ] **9.6 Integration with Pipeline & Strategy**
    - [ ] CEM เป็น shared resource ใน `OrderPipeline` → inject via `Arc<CemManager>`
    - [ ] Strategy ส่ง `StrategyAction` → Pipeline → CEM gate → approve/reject/park
    - [ ] เมื่อ reject: pipeline return `Err` → strategy จัดการ (อิง legacy: park + reevaluate)
    - [ ] เมื่อ exchange event (fill/cancel) → notify CEM เพื่อ unlock capital + reevaluate parked

  - [ ] **9.7 Unit Tests**
    - [ ] Balance virtual lock/unlock
    - [ ] Approve fast path (เงินพอ)
    - [ ] Cancel plan generation (เงินไม่พอ → cancel ตัวไกล)
    - [ ] Smart parking (surplus → เก็บตัวใกล้)
    - [ ] Parked order reevaluation (balance เปลี่ยน → reactivate)
    - [ ] Multi-bot competition (2 bot แย่ง capital asset เดียวกัน)
