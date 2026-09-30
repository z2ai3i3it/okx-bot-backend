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

## Phase 5: Verification & End-to-End Test
- [x] **5.1 ตรวจสอบการคอมไพล์และเทสต์ (`cargo test` - 14 passed)**
- [x] **5.2 ยืนยันการไม่ flood log จาก Ticker (Snap แสดงผลเฉพาะตอนวาง Order เท่านั้น)**
  - [x] Ticker stream ทำงานเงียบใน background ไม่พิมพ์ log ทุก tick
  - [x] Snap log แสดงผลเฉพาะจังหวะ Place Order พร้อมแสดง Original Bid/Ask ครบถ้วน
  - [x] วางคำสั่งคู่ Buy/Sell (+/- 5000 tick) สำเร็จเข้าสู่ Pipeline Chain (Dedup -> CEM -> RateLimit -> OKX Executor)