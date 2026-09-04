# okx-bot-backend

## 🔐 Core Modules: Authentication & OKX Account Linking

> **Branches:** `feat-auth`, `feat/okx-account-linking`
> **Status:** ✅ เสร็จสมบูรณ์ (Auth + Account Linking + AES-256-GCM Encryption)
> **วันที่อัปเดต:** 2026-09-05

### 📋 สิ่งที่ทำเสร็จแล้ว (Completed Modules)

#### 1. ระบบยืนยันตัวตนและจัดการสิทธิ์ (Authentication & RBAC)
| Feature | รายละเอียด | ไฟล์หลัก |
|---------|-----------|----------|
| **Domain Models & Schemas** | นิยาม `User`, `Role` (Admin, Trader, Viewer), `UserStatus`, `Claims`, Auth DTOs | `src/domain/user.rs` |
| **Auth Service Layer** | Argon2id password hashing/verification, JWT (HMAC-SHA256) token generation/verification | `src/users/auth_service.rs` |
| **RBAC & Permission Guard** | Enum `Permission` (20+ granular actions), `PermissionGuard` ตรวจสิทธิ์ตาม Role, tenant data isolation | `src/users/permission.rs` |
| **Axum Middlewares** | `require_auth` (JWT Bearer extraction), `require_admin` (Admin check) | `src/web/middlewares/auth_middleware.rs` |
| **User Persistence** | `UserRepository` สำหรับ MongoDB collection `users` (CRUD operations) | `src/storage/repositories/user_repository.rs` |
| **Auth Handlers** | Register, Login, Get Profile, Update Profile, Change Password, Soft Delete | `src/web/handlers/auth.rs` |

#### 2. ระบบผูกบัญชี OKX API Key & การเข้ารหัส (Exchange Account & AES-256-GCM)
| Feature | รายละเอียด | ไฟล์หลัก |
|---------|-----------|----------|
| **Two-Way Encryption** | AES-256-GCM Authenticated Encryption (สุ่ม Nonce 96-bit ต่อข้อความ, Output เป็น Base64) | `src/crypto/encryption.rs` |
| **Account Domain Model** | `Account` entity, `AccountStatus`, `LinkAccountRequest`, `AccountResponse` พร้อม API Key Masking (`c1b2****90ef`) | `src/domain/account.rs` |
| **Account Persistence** | `AccountRepository` สำหรับ MongoDB collection `accounts` (1 User : N Accounts) | `src/storage/repositories/account_repository.rs` |
| **Account Service** | Link account (encrypt credentials), list accounts, get account, delete account, get decrypted credentials สำหรับ Bot Engine | `src/users/account_service.rs` |
| **Account Handlers** | `POST /api/accounts`, `GET /api/accounts`, `GET /api/accounts/:id`, `DELETE /api/accounts/:id` | `src/web/handlers/account.rs` |
| **OpenAPI / Swagger UI** | Swagger UI (`/swagger-ui`) รองรับ Bearer Auth และ Schema ของทั้ง Auth & Exchange Accounts | `src/web/routes.rs` |

---

### 🛠️ Tech Stack หลัก

| Category | Technology | Version | Description |
|----------|-----------|---------|-------------|
| Web Framework | Axum | 0.8 | Async web framework |
| Runtime | Tokio | 1.0 (full) | Multi-threaded async runtime |
| Database | MongoDB | 3.1 (→ 3.8.2) | Official MongoDB Rust Driver |
| Password Hashing | Argon2id | 0.5 | One-way password hashing |
| Encryption | aes-gcm | 0.10 | Two-way AES-256-GCM for API Keys/Secrets |
| Encoding | base64 | 0.22 | Base64 representation of ciphertexts |
| Token | jsonwebtoken | 9.3 | HMAC-SHA256 Stateless JWT |
| Stream Utility | futures-util | 0.3 | MongoDB Cursor Stream processing |
| Serialization | serde + serde_json | 1.0 | Data serialization & JSON |
| Date/Time | chrono | 0.4 | UTC timestamp tracking |
| UUID | uuid v4 | 1.10 | Unique ID generation |
| API Documentation | utoipa + utoipa-swagger-ui | 5 / 9 | OpenAPI 3.0 Interactive Docs |
| Config | dotenvy | 0.15 | Environment variables loader |

---

### 🌐 API Endpoints Overview

#### 1. Authentication (`/api/auth`)
| Method | Path | Auth | Description |
|--------|------|:----:|-------------|
| `POST` | `/api/auth/register` | Public | สมัครสมาชิกใหม่ → ได้รับ JWT Token |
| `POST` | `/api/auth/login` | Public | เข้าสู่ระบบ → ได้รับ JWT Token |
| `GET` | `/api/auth/me` | Bearer | ดูข้อมูลโปรไฟล์ตนเอง |
| `PUT` | `/api/auth/profile` | Bearer | แก้ไข username / email |
| `PUT` | `/api/auth/password` | Bearer | เปลี่ยนรหัสผ่าน (ต้องยืนยันรหัสผ่านเดิม) |
| `DELETE` | `/api/auth/account` | Bearer | ปิดใช้งานบัญชีผู้ใช้ (Soft Delete → `status: "suspended"`) |

#### 2. Exchange Accounts (`/api/accounts`)
| Method | Path | Auth | Description |
|--------|------|:----:|-------------|
| `POST` | `/api/accounts` | Bearer | ผูกบัญชี OKX API Key ใหม่ (เข้ารหัส AES-256-GCM) |
| `GET` | `/api/accounts` | Bearer | ดูรายการบัญชี OKX ทั้งหมดของผู้ใช้ (แสดงเฉพาะ Masked API Key) |
| `GET` | `/api/accounts/{id}` | Bearer | ดูรายละเอียดบัญชีเดี่ยวตาม ID |
| `DELETE` | `/api/accounts/{id}` | Bearer | ยกเลิกการผูกบัญชี OKX |

#### 3. Documentation & Testing
| Path | Description |
|------|-------------|
| `/swagger-ui` | Swagger UI Interactive API Documentation |
| `/api-docs/openapi.json` | OpenAPI 3.0 JSON specification |

---

### 🏛️ Architecture & Security Decisions

1. **Two-Way Encryption (AES-256-GCM)**:
   - Secret Key และ Passphrase ของ OKX ต้องเข้ารหัสแบบถอดรหัสได้ (Two-way) เพื่อให้บอทเทรดนำไป Sign คำสั่งซื้อขายได้
   - สุ่ม Nonce 96-bit ใหม่ทุกครั้งที่เข้ารหัส ทำให้ข้อความเดิมได้ Ciphertext ต่างกันเสมอ
   - เก็บเฉพาะ `encrypted_secret` และ `encrypted_passphrase` ลงใน Database
2. **API Key Masking**:
   - ไม่มี endpoint ไหนที่ส่ง Secret หรือ Passphrase ออกมาใน Response
   - API Key ที่ส่งกลับทาง API จะถูก Masked ตรงกลางเสมอ (เช่น `c1b2****90ef`)
3. **1 User : N Accounts Relationship**:
   - 1 ผู้ใช้สามารถมีได้หลายบัญชี OKX (เช่น Demo, Grid Bot, Live Portfolio) โดยแยก Collection `accounts` ชัดเจน และอ้างอิงกลับด้วย `user_id`
4. **Soft Delete vs Hard Delete**:
   - การลบบัญชีผู้ใช้ (`/api/auth/account`) ใช้ **Soft Delete** เปลี่ยนสถานะเป็น `suspended` เพื่อรักษา Audit trail และประวัติการเทรด
   - การลบการผูกบัญชี OKX (`DELETE /api/accounts/{id}`) ใช้ **Hard Delete** เพื่อให้ผู้ใช้สามารถถอดถอน API Key ออกจากระบบได้จริง
5. **Config via `.env`**:
   - `ENCRYPTION_KEY`, `JWT_SECRET`, `MONGODB_URI` ถูกแยกเก็บใน `config/secrets.env` ไม่ Hardcode ใน Source code

---

### 📁 Configuration (`config/secrets.env`)

```env
MONGODB_URI=mongodb://localhost:27017
MONGODB_DB_NAME=okx-bot
JWT_SECRET=super-secret-jwt-key-okx-bot-change-me
JWT_EXPIRATION_HOURS=24
PORT=3000
ENCRYPTION_KEY=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
```

---

### 🚀 วิธีรันและทดสอบ

```bash
# ตรวจสอบโค้ด
cargo check

# รันเซิร์ฟเวอร์
cargo run

# เข้าทดสอบผ่าน Swagger UI:
# http://localhost:3000/swagger-ui
```

---

### 🔮 Next Phase (Roadmap ถัดไป)

> **Phase ถัดไป:** Capital Efficiency Manager (CEM) & Pure OKX v5 Connector (WebSocket / REST Client)

| Component | รายละเอียด |
|-----------|-------------|
| **OKX Signer & Connector** | `src/okx/signer.rs`, `src/okx/rest_client.rs`, `src/okx/ws_trade.rs` |
| **CEM Engine** | In-Memory Atomic Balance Tracker & Allocation (`src/capital/ledger.rs`, `allocator.rs`) |
| **Bot Order Pipeline** | Structured Concurrency Order Pipeline Chain (`src/pipeline/`) |

---

## 📂 Project Structure

---

```text
okx-bot-backend/
├── Cargo.toml                          # Rust project dependencies // ไฟล์ระบุ Dependencies ใน Rust (axum, tokio, serde, rust_decimal, ฯลฯ)
├── config/                             # System configuration directory // ไฟล์ตั้งค่าสำหรับสภาพแวดล้อมต่างๆ
│   ├── default.toml                    # Default settings (Web Port, Rate Limit, Stale Threshold) // ค่าตั้งค่าทั่วไป (Web Port, Rate Limit, Stale Threshold)
│   ├── exchange.toml                   # OKX-specific values (Instrument Rules, Tick Size, Lot Size) // กฎของตลาด OKX (Instrument Rules, Qty/Price Step Size)
│   └── secrets.env                     # Secrets (Encryption Key, DB URI, Master Key) // ความลับของระบบ (Encryption Key, DB Connection, Master Pass)
│
└── src/
    ├── main.rs                         # Entry Point (Init DB, Logging, Bot Engine, Web Server) // Entry Point (เชื่อมฐานข้อมูล, โหลด Logging, เปิด Bot Engine, สตาร์ท Web Server)
    ├── config.rs                       # Config Loader (Reads TOML + ENV) // ตัวจัดการโหลด Config & Environment Variables
    │
    ├── domain/                         # 🟢 1. DATA MODELS (Central Schemas — Single Source of Truth) // 🟢 1. DATA MODELS (สคีมาข้อมูลกลาง — Single Source of Truth)
    │   ├── mod.rs                      # Module entry for domain models // ตัวนำเข้าโมดูลโมเดลข้อมูลกลาง
    │   ├── user.rs                     # User Profile, Roles (Admin/Trader), Auth Claims // ข้อมูลผู้ใช้งาน สิทธิ์การเข้าถึง และสคีมาการยืนยันตัวตน
    │   ├── account.rs                  # Exchange Account (API Key, Encrypted Secret, Passphrase, Sub-account) // ข้อมูลการผูกบัญชี OKX API Key ที่เข้ารหัสไว้
    │   ├── strategy.rs                 # ★ Strategy Config & Parameters (StrategyType, Symbol, Interval, Grid Params) // ตั้งค่าการเทรดของบอทและพารามิเตอร์ต่างๆ
    │   ├── order.rs                    # OrderIntent, OrderStatus, Side, OrderType // สถานะของ Order คำสั่งซื้อขายที่ประมวลผลอยู่
    │   ├── trade.rs                    # ★ Matched Trade / Execution Fill Record // บันทึกประวัติการเทรดที่เกิดการจับคู่ซื้อขายจริงบนกระดาน
    │   ├── balance.rs                  # ★ Balance & Position Snapshot (per Asset / per Strategy) // ยอดเงินรวมและโพซิชันสะสมแยกรายบอท/รายเหรียญ
    │   └── events.rs                   # Internal System Event Schemas (Web ↔ Engine communication) // สคีมาของเหตุการณ์ระบบที่ส่งหากันภายในและ WebSockets
    │
    ├── users/                          # 👤 2. USER & AUTHENTICATION (User Management and Security) // 👤 2. USER & AUTHENTICATION (ระบบจัดการผู้ใช้และความปลอดภัย)
    │   ├── mod.rs                      # Module entry for users management // ตัวนำเข้าโมดูลการจัดการผู้ใช้
    │   ├── auth_service.rs             # Register, Login, JWT Token Generation & Verification // บริการสมัครสมาชิก ล็อกอิน และจัดการ JWT Token
    │   ├── account_service.rs          # Manage User's OKX API Keys (Encrypt before store) // บริการเชื่อมต่อ บันทึก หรือลบ OKX API Key โดยเก็บข้อมูลแบบเข้ารหัส
    │   └── permission.rs               # Role-Based Access Control (RBAC) // ระบบตรวจเช็คสิทธิ์การเข้าใช้งานบอทตามความเป็นเจ้าของ
    │
    ├── crypto/                         # 🔐 3. ENCRYPTION SERVICE (★ New — Cryptography Service for Sensitive Data) // 🔐 3. ENCRYPTION SERVICE (★ ใหม่ — ระบบเข้ารหัสข้อมูลลับ)
    │   ├── mod.rs                      # Module entry for cryptography // ตัวนำเข้าโมดูลระบบเข้ารหัส
    │   └── encryption.rs               # AES-256 GCM Encrypt/Decrypt for API Keys & Secrets // การเข้ารหัสและถอดรหัสข้อมูล API Key & Secret ด้วย AES-256 GCM
    │
    ├── capital/                        # 🦁 4. CEM — CAPITAL EFFICIENCY MANAGER (Capital Allocation and Parking) // 🦁 4. CEM — CAPITAL EFFICIENCY MANAGER (ระบบบริหารทุน)
    │   ├── mod.rs                      # Module entry for capital efficiency manager // ตัวนำเข้าโมดูลตัวคุมเงินทุน
    │   ├── ledger.rs                   # In-Memory Atomic Balance Tracker (per Account & Asset) // สมุดบัญชีใน Memory สำหรับบันทึกวงเงินคงเหลือล่าสุดแบบรวดเร็ว
    │   ├── allocator.rs                # Reserve / Release Balance Operations // ระบบล็อก (Reserve) และปลดล็อก (Release) ทุนให้กับบอทต่างๆ
    │   └── planner.rs                  # Smart Parking Engine (Cancel older orders when capital is insufficient) // อัลกอริทึมจัดการ Smart Parking เพื่อสั่งแคนเซิลออร์เดอร์เก่าดึงทุนกลับมาใช้
    │
    ├── pipeline/                       # ⚡ 5. STRUCTURED CONCURRENCY PIPELINE (Order Flow Pipeline Engine) // ⚡ 5. STRUCTURED CONCURRENCY PIPELINE (สายพานลำเลียงคำสั่ง)
    │   ├── mod.rs                      # Module entry for order pipeline // ตัวนำเข้าโมดูลท่อส่งคำสั่งซื้อขาย
    │   ├── context.rs                  # Execution Context (user_id, account_id, CancellationToken) // คอนเท็กซ์ที่ถือข้อมูลผู้ใช้และโทเค็นขอยกเลิกคำสั่งซื้อขาย
    │   ├── order_pipeline.rs           # Declarative Async Pipeline Chain Builder // ตัวเชื่อมร้อยสายพานคำสั่งซื้อขายแบบ Async ทีละสเต็ป
    │   └── steps/                      # Individual steps inside the pipeline // แต่ละ Step บนสายพาน
    │       ├── mod.rs                  # Module entry for pipeline steps // ตัวนำเข้าโมดูลขั้นตอนสายพาน
    │       ├── dedup.rs                # Step 1: Filter out duplicate events within ms // ขั้นตอนตัดเหตุการณ์ที่ถูกยิงซ้ำซ้อนภายในระดับมิลลิวินาที
    │       ├── cem_step.rs             # Step 2: CEM Gatekeeper (Check and lock capital) // ขั้นตอนตรวจสอบและล็อกเงินทุนผ่าน CEM
    │       ├── rate_limit.rs           # Step 3: Token Bucket Rate Limit check // ขั้นตอนตรวจสอบโควต้าเพื่อป้องกันการยิงเกินข้อจำกัดของ OKX
    │       └── okx_executor.rs         # Step 4: Dispatch directly to OKX WS Trade API // ขั้นตอนส่งข้อมูลคำสั่งซื้อขายเข้าสู่ท่อ OKX WebSocket Trade
    │
    ├── okx/                            # 🔴 6. PURE OKX v5 CONNECTOR (Direct Connection to OKX v5 API) // 🔴 6. PURE OKX v5 CONNECTOR (เชื่อมต่อ OKX โดยตรง)
    │   ├── mod.rs                      # Module entry for OKX engine // ตัวนำเข้าโมดูลเชื่อมต่อ OKX
    │   ├── manager.rs                  # Connection Pool Controller (WS Pool per sub-account) // ตัวจัดการและควบคุมการเปิด WebSocket Connections แยกตามบัญชีลูก
    │   ├── signer.rs                   # HMAC-SHA256 Authentication Signer // ตัวคำนวณการเข้ารหัส HMAC-SHA256 และจัดเตรียม Timestamp สำหรับตรวจสอบสิทธิ์
    │   ├── rest_client.rs              # HTTP REST Client (History Sync, Fallback) // ตัวเชื่อมต่อยิง HTTP REST สำหรับงานทั่วไปหรือการกู้ประวัติ
    │   ├── ws_client.rs                # WebSocket Reader (Orderbook, Ticker, Order Fills) // ตัวเชื่อมต่ออ่านข้อมูลราคาตลาดและอ่านข้อมูล Order Fills สดจาก WS
    │   ├── ws_trade.rs                 # WebSocket Trade API (Place/Cancel Orders — Ultra-Low Latency) // ตัวยิงคำสั่งซื้อขายด่วนพิเศษผ่าน WS Trade API เพื่อความเร็วสูงสุด
    │   ├── rate_limiter.rs             # OKX Weight-based Token Bucket Engine // ตัวนับสิทธิ์การยิงตามน้ำหนัก weight ของแต่ละประเภทคำสั่งใน OKX
    │   └── dto/                        # OKX JSON Payload Structs // สเตติกสคีมาข้อมูล JSON ตามโครงสร้างจริงของ OKX API
    │       ├── mod.rs                  # Module entry for Data Transfer Objects // ตัวนำเข้าโมดูลรับส่งข้อมูล OKX
    │       ├── account.rs              # OKX specific account DTOs // รูปแบบข้อมูลเกี่ยวกับระบบบัญชี
    │       ├── market.rs               # OKX specific market DTOs // รูปแบบข้อมูลเกี่ยวกับราคาและข้อมูลตลาด
    │       └── trade.rs                # OKX specific trade DTOs // รูปแบบข้อมูลเกี่ยวกับการยิงออร์เดอร์และยกเลิกออร์เดอร์
    │
    ├── bot/                            # 🤖 7. TRADING ENGINE & STRATEGY MANAGER (Bot Lifecycle & Management) // 🤖 7. TRADING ENGINE & STRATEGY MANAGER (บอทและตัวคุมกลยุทธ์)
    │   ├── mod.rs                      # Module entry for trading bot manager // ตัวนำเข้าโมดูลควบคุมบอท
    │   ├── manager.rs                  # Bot Lifecycle Controller (Start/Stop/Pause per User/Account) // ตัวคุมคำสั่งเปิดปิดการทำงานของบอทเทรด
    │   ├── executor.rs                 # ★ Strategy Executor (Runs Strategy Loop & WS subscription) // ตัวรับผิดชอบรันลูปกลยุทธ์และจัดเตรียมการรับข้อมูล WS
    │   ├── scheduler.rs                # ★ Background Task Scheduler (Periodic Health Check, Auto-Restart) // ตัวตั้งเวลาจัดงานเบื้องหลัง เช่น ตรวจสุขภาพบอท หรือปลุกบอทที่ดับ
    │   ├── order_tracker.rs            # In-Memory Active Orders Tracker // ตัวติดตามจำข้อมูลออร์เดอร์ที่ยังเปิดค้างอยู่บนกระดานใน Memory
    │   ├── recovery.rs                 # ★ Recovery Engine (Restore Bot state after restart/crash) // ระบบเก็บกู้สถานะเดิมของบอทหลังระบบดับหรือพังลง
    │   │
    │   ├── strategies/                 # Trading Strategy Logic // อัลกอริทึมการเทรด
    │   │   ├── mod.rs                  # Module entry for strategies // ตัวนำเข้าโมดูลกลยุทธ์
    │   │   ├── base.rs                 # Strategy Trait / Interface specification // ตัวนิยามอินเตอร์เฟซและเมทอดหลักของกลยุทธ์ (Trait)
    │   │   └── fixed_ratio.rs          # Fixed Ratio Grid Strategy Implementation // ตัวจัดการบอท Fixed Ratio Grid Strategy
    │   │
    │   └── math/                       # 🧮 MATHEMATICAL PURE CALCULATIONS (★ New — Pure Math Calculations for Strategies) // 🧮 MATHEMATICAL PURE CALCULATIONS (★ ใหม่ — สูตรคณิตศาสตร์แยกส่วน)
    │       ├── mod.rs                  # Module entry for mathematical calculations // ตัวนำเข้าโมดูลคณิตศาสตร์
    │       ├── fixed_ratio_cal.rs      # Pure math formulas for Fixed Ratio target price, target quantity, rounding // อัลกอริทึมสูตรคณิตศาสตร์คำนวณราคาและจำนวนที่ควรตั้งซื้อขายของ Fixed Ratio
    │       └── grid_cal.rs             # Pure math formulas for Grid Spacing calculations // อัลกอริทึมสูตรคณิตศาสตร์สำหรับกลยุทธ์กริดทั่วไป
    │
    ├── web/                            # 🌐 8. WEB SERVER & DASHBOARD BACKEND (Axum Web Application API) // 🌐 8. WEB SERVER & DASHBOARD BACKEND (AXUM)
    │   ├── mod.rs                      # Module entry for web server // ตัวนำเข้าโมดูลเว็บเซิร์ฟเวอร์
    │   ├── state.rs                    # Shared AppState (Internal Broadcast & mpsc Channels) // สถานะแชร์ร่วมกันและตัวจัดการช่องทาง broadcast สัญญาณภายในเว็บ
    │   ├── middlewares/                # HTTP Middlewares // ตัวดักฟิลเตอร์คำขอเข้าเว็บ
    │   │   ├── mod.rs                  # Module entry for middlewares // ตัวนำเข้าโมดูลตัวดักกรองเว็บ
    │   │   ├── auth_middleware.rs      # JWT Token Validation Middleware // ตัวกรองตรวจสอบความถูกต้องของรหัสสิทธิ์ JWT Token
    │   │   └── cors.rs                 # ★ CORS Configuration for Frontend Origin integration // จัดตั้งนโยบายให้หน้าเว็บติดต่อข้ามเครือข่ายเข้าเซิร์ฟเวอร์บอทได้
    │   ├── routes.rs                   # API Endpoints Registry // สารบัญระบุพิกัด URL API ทั้งหมดของโปรเจกต์
    │   ├── handlers/                   # REST API Handlers // ฟังก์ชันดำเนินการเมื่อรับส่งข้อมูลทาง HTTP Request
    │   │   ├── mod.rs                  # Module entry for handlers // ตัวนำเข้าโมดูลตัวจัดการ API
    │   │   ├── auth.rs                 # User Login and Registration API Handlers // ฟังก์ชันย่อยสำหรับเข้าใช้งานระบบผู้ใช้
    │   │   ├── account.rs              # OKX API Key integration API Handlers // ฟังก์ชันย่อยสำหรับจัดการผูก OKX API Keys
    │   │   ├── bot_control.rs          # Bot control (Start, Stop, Pause) API Handlers // ฟังก์ชันย่อยสำหรับส่งคำสั่งควบคุมบอทเทรด
    │   │   ├── strategy_config.rs      # Strategy parameter editing API Handlers // ฟังก์ชันย่อยสำหรับตั้งค่าพารามิเตอร์บอท
    │   │   ├── analytics.rs            # PnL performance and Trade History API Handlers // ฟังก์ชันย่อยสำหรับสรุปผลกำไรขาดทุน PnL
    │   │   └── monitoring.rs           # ★ Performance Metrics and latency stats API Handlers // ฟังก์ชันย่อยเรียกดูความเร็วการประมวลผลระบบ
    │   └── ws_dashboard.rs             # Real-time WebSocket Feed to Frontend UI dashboard // ท่อส่งกระจายข้อมูลสดแบบเรียลไทม์ส่งตรงเข้าแดชบอร์ดหน้าเว็บ
    │
    ├── services/                       # 🧩 9. BUSINESS LOGIC SERVICES (★ New — Application Core Logic Layer) // 🧩 9. BUSINESS LOGIC SERVICES (★ ใหม่ — Application Layer กลาง)
    │   ├── mod.rs                      # Module entry for business services // ตัวนำเข้าโมดูลบริการหลักระบบ
    │   ├── strategy_service.rs         # Strategy CRUD Logic & Lifecycle Validation // ตรรกะจัดการเพิ่มลบแก้ไขตั้งค่ากลยุทธ์บอท
    │   ├── order_service.rs            # Order History Query & Aggregation Logic // ตรรกะจัดการประวัติการสั่งออร์เดอร์
    │   ├── trade_service.rs            # Trade Fill Processing & PnL Calculation // ตรรกะประมวลผลออร์เดอร์ที่จับคู่จริงและคำนวณกำไร
    │   ├── rebalance_service.rs        # ★ Rebalance Calculation & Execution Logic // ตรรกะประมวลผลคำนวณและเก็บสถิติการรีบาลานซ์พอร์ต
    │   └── balance_service.rs          # ★ Strategy-level Balance Snapshot Logic // ตรรกะจัดการบันทึกและตรวจสอบยอดเงินในกลยุทธ์
    │
    ├── observability/                  # 📊 10. MONITORING & OBSERVABILITY (★ New — Metrics & Structured Logging) // 📊 10. MONITORING & OBSERVABILITY (★ ใหม่ — Metrics & Logging)
    │   ├── mod.rs                      # Module entry for observability // ตัวนำเข้าโมดูลบันทึกและจับความเร็วระบบ
    │   ├── metrics.rs                  # Performance Metrics Collector (Latency and Throughput) // ตัวจับเวลาเก็บข้อมูลสถิติความหน่วง Latency ของระบบ
    │   └── logging.rs                  # Structured Logging Setup (Microsecond precision log routing) // การจัดรูปแบบล็อกระบบแบบละเอียดบันทึกเวลาเป็นไมโครวินาที
    │
    └── storage/                        # 💾 11. PERSISTENCE & DATABASE LAYER (MongoDB Drivers & Repositories) // 💾 11. PERSISTENCE & DATABASE LAYER (MongoDB)
        ├── mod.rs                      # Module entry for storage // ตัวนำเข้าโมดูลฐานข้อมูล
        ├── db.rs                       # Database Connection Pool Manager // ตัวจัดการเชื่อมต่อฐานข้อมูลหลัก MongoDB
        └── repositories/               # Repository Pattern (Clean Data Access — No raw queries outside) // รูปแบบการแยกฟังก์ชันทำคำสั่งติดต่อ Database
            ├── mod.rs                  # Module entry for repositories // ตัวนำเข้าโมดูลคิวรีข้อมูล
            ├── user_repository.rs      # User Profiles & Password Hashes repository queries // คิวรีเขียนอ่านประวัติผู้ใช้
            ├── account_repository.rs   # OKX API Keys (Encrypted Storage) repository queries // คิวรีเขียนอ่าน API Key ที่เข้ารหัสไว้
            ├── strategy_repository.rs  # ★ Strategy Configurations & Running State repository queries // คิวรีเขียนอ่านข้อมูลกลยุทธ์และสถานะวิ่งของบอท
            ├── order_repository.rs     # Order History Records repository queries // คิวรีเขียนอ่านข้อมูลประวัติส่งออร์เดอร์
            ├── trade_repository.rs     # Matched Trade Executions repository queries // คิวรีเขียนอ่านข้อมูลการเทรดที่ match สำเร็จ
            └── rebalance_repository.rs # ★ Rebalance History Records repository queries // คิวรีเขียนอ่านประวัติขั้นตอนการรีบาลานซ์
```
