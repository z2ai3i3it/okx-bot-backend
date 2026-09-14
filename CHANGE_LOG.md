# [CHANGE_LOG] 2026-09-03 - Implement User Domain Models & Auth Schemas

## 1. META_DATA
- **Feature/Issue:** User Authentication & Authorization Domain Models
- **Target Component:** `domain/user.rs`, `domain/account.rs`, `domain/mod.rs`, `Cargo.toml`
- **Action Type:** ADD | MODIFY

## 2. MODIFIED_FILES
- `Cargo.toml`: เพิ่ม dependencies `serde` (derive) และ `chrono` (serde)
- `src/domain/user.rs`: นิยาม User entity, Role, UserStatus, Claims และ Auth DTOs
- `src/domain/account.rs`: เพิ่ม derive macros (`Debug`, `Clone`, `Serialize`, `Deserialize`)
- `src/domain/mod.rs`: expose โมดูล `user` และ `account`

## 3. CONTEXT_AND_REASON
- **Problem/Requirement:** ต้องการวางรากฐาน Data Models ของระบบ Authentication & Authorization และ User Profile ก่อนเริ่ม implement auth service และ middleware
- **Previous Behavior:** ไฟล์ `src/domain/user.rs` มีเพียง Struct แบบเริ่มต้นที่ยังไม่มี password hash, timestamps, roles และ DTOs สำหรับการทำงานจริง

## 4. IMPLEMENTATION_DETAILS
- **[ADDED]:**
  - Enum `Role` (`Admin`, `Trader`, `Viewer`) พร้อม Default เป็น `Trader`
  - Enum `UserStatus` (`Active`, `Suspended`, `PendingVerification`) พร้อม Default เป็น `Active`
  - Struct `Claims` สำหรับ payload ของ JWT Token (`sub`, `username`, `role`, `exp`, `iat`)
  - Struct DTOs สำหรับ Web API: `RegisterRequest`, `LoginRequest`, `UserResponse`, `AuthResponse`
  - Helper methods บน `User`: `User::new()`, `is_admin()`, `is_active()`, `to_response()`
- **[MODIFIED]:**
  - อัปเดต `User` struct ให้มี `password_hash` (`#[serde(skip_serializing)]`) เพื่อความปลอดภัย, และเพิ่มฟิลด์ Timestamps (`created_at`, `updated_at`, `last_login_at`)
  - เพิ่ม derive `Serialize`, `Deserialize`, `Debug`, `Clone` ให้กับ `Account` struct ใน `src/domain/account.rs`
- **[DEPRECATED/REMOVED]:** ไม่มี

## 5. BREAKING_CHANGES_AND_SIDE_EFFECTS
- **Breaking Changes:** NO (เป็นการเริ่มต้น implement บน branch `feat-auth`)
- **Dependencies Added:**
  - `serde = { version = "1.0", features = ["derive"] }`
  - `chrono = { version = "0.4", features = ["serde"] }`

## 6. EXPECTED_BEHAVIOR
- มีโมเดลกลางสำหรับจัดการข้อมูลผู้ใช้, สิทธิ์การใช้งาน (RBAC), และโครงสร้าง JWT Claims พร้อมใช้งานในโมดูล `users/auth_service.rs` และ `web/middlewares/auth_middleware.rs`
- ข้อมูล Password hash จะถูกป้องกันไม่ให้หลุดออกไปทาง JSON response โดยอัตโนมัติ

---

# [CHANGE_LOG] 2026-09-03 - Implement AuthService (Argon2 Hashing & JWT Token Generation)

## 1. META_DATA
- **Feature/Issue:** User Authentication Service Layer
- **Target Component:** `users/auth_service.rs`, `users/mod.rs`, `Cargo.toml`
- **Action Type:** ADD | MODIFY

## 2. MODIFIED_FILES
- `Cargo.toml`: เพิ่ม dependencies `jsonwebtoken`, `argon2`, `rand_core`, `thiserror`
- `src/users/auth_service.rs`: สร้าง service ตรรกะ hash password, verify password, สร้าง/ตรวจสอบ JWT token, และตรวจสอบ credential
- `src/users/mod.rs`: expose โมดูล `auth_service`

## 3. CONTEXT_AND_REASON
- **Problem/Requirement:** ต้องการ Service Layer เพื่อจัดการกระบวนการยืนยันตัวตน (Authentication) ตั้งแต่การเข้ารหัสผ่านอย่างปลอดภัย ไปจนถึงการออกและตรวจรับรอง JWT token
- **Previous Behavior:** ไฟล์ `src/users/auth_service.rs` ยังว่างเปล่า ไม่มี business logic

## 4. IMPLEMENTATION_DETAILS
- **[ADDED]:**
  - Enum `AuthError` จัดการ Error cases (`InvalidCredentials`, `UserAlreadyExists`, `AccountInactive`, `HashError`, `TokenError`, `ValidationError`)
  - Struct `AuthConfig` สำหรับตั้งค่า JWT secret และ Token expiration
  - Struct `AuthService` พร้อม methods:
    - `hash_password()`: เข้ารหัสรหัสผ่านด้วย Argon2id พร้อมสุ่ม Salt
    - `verify_password()`: ตรวจสอบความถูกต้องของรหัสผ่านเทียบกับ Argon2 hash
    - `generate_token()`: สร้าง JWT Access Token ที่มี User ID, Username, Role, Expiration
    - `verify_token()`: ตรวจสอบความถูกต้องและถอดรหัส Token ออกมาเป็น `Claims`
    - `create_user_entity()`: Validate input และสร้าง `User` entity พร้อมรหัสผ่านที่ถูก hash
    - `authenticate_user()`: ตรวจสอบสถานะ User, ตรวจรหัสผ่าน และออก `AuthResponse`
  - Unit tests ครอบคลุมการ hash/verify password และ generate/verify JWT token
- **[MODIFIED]:** ไม่มี
- **[DEPRECATED/REMOVED]:** ไม่มี

## 5. BREAKING_CHANGES_AND_SIDE_EFFECTS
- **Breaking Changes:** NO
- **Dependencies Added:**
  - `jsonwebtoken = "9.3"`
  - `argon2 = { version = "0.5", features = ["std"] }`
  - `rand_core = { version = "0.6", features = ["std"] }`
  - `thiserror = "2.0"`

## 6. EXPECTED_BEHAVIOR
- ระบบสามารถแฮชรหัสผ่านด้วยมาตรฐาน Argon2id และตรวจรับรองรหัสผ่านได้อย่างถูกต้อง ปลอดภัย
- ระบบสามารถสร้าง JWT Access Token และตรวจสอบ Claims (รวมถึง Role สำหรับ RBAC) ได้อย่างถูกต้อง
- พร้อมนำไปเชื่อมต่อเข้ากับ Axum Auth Middleware และ Web Handlers ต่อไป

---

# [CHANGE_LOG] 2026-09-03 - Implement Axum Auth & Admin Middlewares

## 1. META_DATA
- **Feature/Issue:** Web Authentication & Role Authorization Middleware
- **Target Component:** `web/middlewares/auth_middleware.rs`, `web/state.rs`, `web/middlewares/mod.rs`, `web/mod.rs`, `Cargo.toml`
- **Action Type:** ADD | MODIFY

## 2. MODIFIED_FILES
- `Cargo.toml`: เพิ่ม dependencies `axum`, `tokio`, `tower`, `serde_json`
- `src/web/state.rs`: สร้าง `AppState` สำหรับแชร์ `AuthService` ข้าม handlers และ middlewares
- `src/web/middlewares/auth_middleware.rs`: สร้าง middleware `require_auth` (ตรวจ JWT Bearer) และ `require_admin` (ตรวจ RBAC Admin role)
- `src/web/middlewares/mod.rs`: expose โมดูล `auth_middleware`
- `src/web/mod.rs`: expose โมดูล `middlewares` และ `state`

## 3. CONTEXT_AND_REASON
- **Problem/Requirement:** ต้องการ Middleware ดักกรองคำขอ (Request) เข้าสู่ Web API เพื่อบังคับยืนยันตัวตนด้วย JWT Token และตรวจสอบสิทธิ์ผู้ใช้ก่อนถึง Business Handlers
- **Previous Behavior:** ไฟล์ `auth_middleware.rs` และ `state.rs` ยังว่างเปล่า ไม่มี middleware logic

## 4. IMPLEMENTATION_DETAILS
- **[ADDED]:**
  - Struct `AppState` บรรจุ `Arc<AuthService>` สำหรับใช้ร่วมกันใน Axum State Extractor
  - Middleware `require_auth`:
    - ตรวจสอบ `Authorization: Bearer <token>` Header
    - ถอดรหัสและตรวจสอบความถูกต้องของ Token ผ่าน `AuthService::verify_token()`
    - แนบ `Claims` ลงใน request extensions (`req.extensions_mut().insert(claims)`) ให้ Handlers ใช้งานต่อได้
    - ส่งกลับ `401 Unauthorized` (JSON format) ทันทีหาก Header หายไปหรือไม่ถูกต้อง/หมดอายุ
  - Middleware `require_admin`:
    - ตรวจสอบว่า `claims.role == Role::Admin`
    - ส่งกลับ `403 Forbidden` หากไม่ใช่ Admin
- **[MODIFIED]:** ไม่มี
- **[DEPRECATED/REMOVED]:** ไม่มี

## 5. BREAKING_CHANGES_AND_SIDE_EFFECTS
- **Breaking Changes:** NO
- **Dependencies Added:**
  - `axum = "0.8"`
  - `tokio = { version = "1.0", features = ["full"] }`
  - `tower = { version = "0.5", features = ["util"] }`
  - `serde_json = "1.0"`

## 6. EXPECTED_BEHAVIOR
- Protected Endpoints สามารถใช้ `.route_layer(axum::middleware::from_fn_with_state(state, require_auth))` เพื่อดักกรอง Request ที่ไม่มีหรือ Token ไม่ถูกต้อง
- Admin Endpoints สามารถใช้ `require_admin` ซ้อนเพื่อจำกัดสิทธิ์เฉพาะผู้ดูแลระบบ

---

# [CHANGE_LOG] 2026-09-03 - Implement Auth Web Handlers, Router, and Server Entry Point

## 1. META_DATA
- **Feature/Issue:** Web API Auth Endpoints & Routing
- **Target Component:** `web/handlers/auth.rs`, `web/routes.rs`, `web/handlers/mod.rs`, `web/mod.rs`, `main.rs`, `Cargo.toml`
- **Action Type:** ADD | MODIFY

## 2. MODIFIED_FILES
- `Cargo.toml`: เพิ่ม dependency `uuid = { version = "1.10", features = ["v4"] }`
- `src/web/handlers/auth.rs`: REST API handlers สำหรับ `register`, `login`, และ `get_current_user`
- `src/web/handlers/mod.rs`: expose โมดูล `auth`
- `src/web/routes.rs`: รวม Route endpoints (`/api/auth/...`) และผูก middleware `require_auth`
- `src/web/mod.rs`: expose โมดูล `handlers` และ `routes`
- `src/main.rs`: entry point สตาร์ท Axum Web Server ผูกกับ AppState และ Router

## 3. CONTEXT_AND_REASON
- **Problem/Requirement:** ต้องการ HTTP API endpoints ให้ Frontend สามารถส่ง request เข้ามาสมัครสมาชิก, ล็อกอิน, และดึงข้อมูล Profile ของตนเองผ่าน Token
- **Previous Behavior:** ไฟล์ handlers, routes และ main.rs ยังไม่มี logic การทำงานจริง

## 4. IMPLEMENTATION_DETAILS
- **[ADDED]:**
  - Handlers ใน `src/web/handlers/auth.rs`:
    - `register`: รับ `RegisterRequest` -> สร้าง User ใหม่ด้วย UUID -> ออก Token -> ส่งกลับ `201 Created`
    - `login`: รับ `LoginRequest` -> ตรวจสอบ Credential -> ส่งกลับ `200 OK` พร้อม `AuthResponse`
    - `get_current_user`: Protected endpoint รับ `Extension(claims)` -> ส่งข้อมูล Profile ของเจ้าของ Token กลับ
    - `handle_auth_error`: ฟังก์ชันแปลง `AuthError` ให้เป็น HTTP status codes (400, 401, 403, 404, 409, 500) ในรูปแบบ JSON
  - Routing ใน `src/web/routes.rs`:
    - Public: `POST /api/auth/register`, `POST /api/auth/login`
    - Protected: `GET /api/auth/me` (ครอบด้วย `require_auth` middleware)
  - เซิร์ฟเวอร์ใน `src/main.rs`: ผูก `TcpListener` เข้ากับ Axum Server ที่พอร์ต `127.0.0.1:3000`
- **[MODIFIED]:** ไม่มี
- **[DEPRECATED/REMOVED]:** ไม่มี

## 5. BREAKING_CHANGES_AND_SIDE_EFFECTS
- **Breaking Changes:** NO
- **Dependencies Added:**
  - `uuid = { version = "1.10", features = ["v4"] }`

## 6. EXPECTED_BEHAVIOR
- ไคลเอนต์สามารถส่ง `POST /api/auth/register` เพื่อรับ Token แรกเริ่มได้ทันที
- ไคลเอนต์สามารถส่ง `POST /api/auth/login` เพื่อขอรับ Access Token
- ไคลเอนต์สามารถเรียก `GET /api/auth/me` โดยแนบ `Authorization: Bearer <token>` เพื่อดูข้อมูลตนเองได้ หากไม่มี Token จะได้ `401 Unauthorized`

---

# [CHANGE_LOG] 2026-09-03 - Implement Role-Based Access Control (RBAC) & Ownership Guard

## 1. META_DATA
- **Feature/Issue:** User Permission & Resource Ownership Policy
- **Target Component:** `users/permission.rs`, `users/mod.rs`
- **Action Type:** ADD | MODIFY

## 2. MODIFIED_FILES
- `src/users/permission.rs`: สร้าง Enum `Permission` และ Struct `PermissionGuard`
- `src/users/mod.rs`: expose โมดูล `permission`

## 3. CONTEXT_AND_REASON
- **Problem/Requirement:** ต้องการระบบตรวจสอบสิทธิ์ระดับ Granular Action (RBAC) และการตรวจสอบความเป็นเจ้าของทรัพยากร (Resource Ownership Check) เช่น ป้องกัน Trader ไม่ให้สั่งหยุดบอทหรือเข้าถึง API Key ของ Trader คนอื่น
- **Previous Behavior:** ไฟล์ `permission.rs` ยังว่างเปล่า ไม่มี logic ตรวจสอบสิทธิ์

## 4. IMPLEMENTATION_DETAILS
- **[ADDED]:**
  - Enum `Permission`:
    - จัดการสิทธิ์การควบคุมผู้ใช้ (`ManageUsers`, `ViewUsers`)
    - จัดการสิทธิ์ OKX Account (`LinkExchangeAccount`, `ViewExchangeAccount`, `DeleteExchangeAccount`)
    - จัดการสิทธิ์ Bot Control (`StartBot`, `StopBot`, `PauseBot`, `ConfigureStrategy`)
    - จัดการสิทธิ์ดูสถิติและการเงิน (`ViewPnL`, `ViewTradeHistory`, `ViewSystemMetrics`)
  - Struct `PermissionGuard`:
    - `get_permissions_by_role()`: แมปรายการสิทธิ์ตาม `Role` (`Admin`, `Trader`, `Viewer`)
    - `has_permission()`: ตรวจสอบความถูกต้องของ Role กับ Permission
    - `can()`: ตรวจสอบตรงจาก `Claims` ของ JWT
    - `can_access_resource()`: ตรวจสอบว่าผู้ใช้ปัจจุบันเป็น Admin หรือเป็นเจ้าของทรัพยากรชิ้นนั้นจริง
  - Unit tests ครอบคลุมการเช็คสิทธิ์ตาม Role ต่างๆ และการเช็ค Tenant Resource Ownership
- **[MODIFIED]:** ไม่มี
- **[DEPRECATED/REMOVED]:** ไม่มี

## 5. BREAKING_CHANGES_AND_SIDE_EFFECTS
- **Breaking Changes:** NO
- **Dependencies Added:** ไม่มี

## 6. EXPECTED_BEHAVIOR
- โมดูล Bot Control และ Account Services สามารถเรียก `PermissionGuard::can(&claims, &Permission::StartBot)` เพื่อตรวจสิทธิ์ก่อนดำเนินการได้
- ระบบสามารถป้องกันการเข้าถึงข้อมูลข้ามบัญชีได้อย่างปลอดภัย (Multi-tenant data isolation)

---

# [CHANGE_LOG] 2026-09-03 - Implement MongoDB Persistence Layer & Configuration Loader (Phase 1)

## 1. META_DATA
- **Feature/Issue:** MongoDB Database Integration for User Persistence & Config Management
- **Target Component:** `storage/db.rs`, `storage/repositories/user_repository.rs`, `config.rs`, `config/secrets.env`, `web/handlers/auth.rs`, `main.rs`, `Cargo.toml`
- **Action Type:** ADD | MODIFY

## 2. MODIFIED_FILES
- `Cargo.toml`: เพิ่ม dependencies `mongodb`, `bson`, `dotenvy`
- `config/secrets.env`: กำหนด `MONGODB_URI`, `MONGODB_DB_NAME`, `JWT_SECRET`, `PORT`
- `src/config.rs`: สร้าง `AppConfig` โหลดการตั้งค่าจาก ENV และไฟล์ `secrets.env`
- `src/storage/db.rs`: สร้างฟังก์ชัน `init_db` เชื่อมต่อ MongoDB Client
- `src/storage/repositories/user_repository.rs`: สร้าง `UserRepository` บันทึก/ค้นหา/อัปเดต/ลบ User ลง collection `users`
- `src/storage/repositories/mod.rs` & `src/storage/mod.rs`: expose storage modules
- `src/web/state.rs`: เพิ่ม `user_repo: Arc<UserRepository>` ใน `AppState`
- `src/web/handlers/auth.rs`: แทนที่ Mock ด้วยการบันทึก User จริงลง DB และตรวจสอบ duplicate email/username
- `src/main.rs`: เชื่อมต่อ MongoDB ตั้งแต่ตอนสตาร์ทเซิร์ฟเวอร์

## 3. CONTEXT_AND_REASON
- **Problem/Requirement:** เดิมระบบ Auth ใช้ข้อมูลจำลอง (Mock) ใน Memory ทำให้ข้อมูลผู้ใช้หายไปเมื่อรีสตาร์ท และไม่สามารถบันทึกผู้ใช้จริงได้ จึงต้องเชื่อมต่อ MongoDB เพื่อรองรับการทำงานจริง
- **Previous Behavior:** ฟังก์ชัน Register และ Login ใน Handlers เป็น Mock logic

## 4. IMPLEMENTATION_DETAILS
- **[ADDED]:**
  - ฟังก์ชัน `init_db(uri, db_name)` จัดการ Connection Pool ของ MongoDB
  - `UserRepository` พร้อม methods: `create`, `find_by_id`, `find_by_email`, `find_by_username`, `update`, `delete_by_id` สำหรับ collection `users`
  - Struct `AppConfig` โหลดค่าคอนฟิกจาก `config/secrets.env` (DB Name: `okx-bot`)
- **[MODIFIED]:**
  - Handlers `register`, `login`, `get_current_user` เชื่อมโยงเข้ากับ MongoDB `UserRepository` จริง
  - ตรวจสอบความซ้ำซ้อนของ Email และ Username ก่อนบันทึก
- **[DEPRECATED/REMOVED]:** ลบ Dummy User Mock ออกทั้งหมด

## 5. BREAKING_CHANGES_AND_SIDE_EFFECTS
- **Breaking Changes:** NO
- **Dependencies Added:**
  - `mongodb = { version = "3.1", default-features = false, features = ["tokio-runtime"] }`
  - `bson = { version = "2.13", features = ["chrono-0_4"] }`
  - `dotenvy = "0.15"`

## 6. EXPECTED_BEHAVIOR
- เมื่อเซิร์ฟเวอร์สตาร์ท จะเชื่อมต่อกับ MongoDB ที่ `mongodb://localhost:27017` ฐานข้อมูล `okx-bot`
- เมื่อเรียก `POST /api/auth/register` ข้อมูล User พร้อม password hash จะถูกบันทึกจริงลง Collection `users`
- เมื่อเรียก `POST /api/auth/login` ระบบจะค้นหา User จริงจาก DB และตรวจรหัสผ่าน หากถูกต้องจะคืนค่า Token
- ข้อมูลสามารถเปิดดูและจัดการผ่านโปรแกรม MongoDB Compass ได้ทันที

---

# [CHANGE_LOG] 2026-09-03 - Implement User CRUD (Profile, Password Change, Soft Delete) (Phase 2)

## 1. META_DATA
- **Feature/Issue:** Complete User Profile & Security CRUD Endpoints
- **Target Component:** `domain/user.rs`, `users/auth_service.rs`, `web/handlers/auth.rs`, `web/routes.rs`
- **Action Type:** ADD | MODIFY

## 2. MODIFIED_FILES
- `src/domain/user.rs`: เพิ่ม DTOs `UpdateProfileRequest` และ `ChangePasswordRequest`
- `src/users/auth_service.rs`: เพิ่มเมทอด `process_change_password` (ตรวจรหัสผ่านเดิม และสร้าง hash ใหม่)
- `src/web/handlers/auth.rs`: เพิ่ม handlers `update_profile`, `change_password`, `delete_account`
- `src/web/routes.rs`: ลงทะเบียน routes `PUT /profile`, `PUT /password`, `DELETE /account` ภายใต้ auth protected scope

## 3. CONTEXT_AND_REASON
- **Problem/Requirement:** ต้องการให้ผู้ใช้ที่ล็อกอินแล้วสามารถจัดการข้อมูลส่วนตัวได้ครบถ้วน (อัปเดต Username/Email, เปลี่ยนรหัสผ่าน, และขอยกเลิก/ปิดการใช้งานบัญชี)
- **Previous Behavior:** มีเฉพาะ Register, Login, และ Get Current User

## 4. IMPLEMENTATION_DETAILS
- **[ADDED]:**
  - DTOs `UpdateProfileRequest` (username, email) และ `ChangePasswordRequest` (old_password, new_password)
  - เมทอด `AuthService::process_change_password(&user, old_pass, new_pass)`
  - Endpoint `PUT /api/auth/profile`:
    - ตรวจสอบความถูกต้องของ Email/Username ใหม่ และเช็คความซ้ำซ้อนกับผู้ใช้อื่นใน MongoDB
    - อัปเดตข้อมูลและ Timestamp `updated_at`
  - Endpoint `PUT /api/auth/password`:
    - ตรวจสอบรหัสผ่านเดิมก่อน หากถูกต้องจะแฮชรหัสผ่านใหม่ด้วย Argon2id และบันทึก
  - Endpoint `DELETE /api/auth/account`:
    - ปิดการใช้งานบัญชีในรูปแบบ Soft Delete โดยเปลี่ยนสถานะ `status` เป็น `UserStatus::Suspended`
- **[MODIFIED]:**
  - `src/web/routes.rs`: ผูก routes เข้ากับ `require_auth` middleware
- **[DEPRECATED/REMOVED]:** ไม่มี

## 5. BREAKING_CHANGES_AND_SIDE_EFFECTS
- **Breaking Changes:** NO
- **Dependencies Added:** ไม่มี

## 6. EXPECTED_BEHAVIOR
- ผู้ใช้ที่มี JWT Token สามารถเรียก `PUT /api/auth/profile` เพื่อแก้ไขชื่อหรืออีเมลได้
- ผู้ใช้สามารถเรียก `PUT /api/auth/password` เพื่อเปลี่ยนรหัสผ่านได้
- ผู้ใช้สามารถเรียก `DELETE /api/auth/account` เพื่อปิดการใช้งานบัญชีตนเองได้ทันที โดยสถานะใน MongoDB Compass จะเปลี่ยนเป็น `suspended`

---

# [CHANGE_LOG] 2026-09-03 - Integrate OpenAPI & Swagger UI (utoipa)

## 1. META_DATA
- **Feature/Issue:** Swagger UI & OpenAPI 3.0 Documentation for Web API
- **Target Component:** `web/routes.rs`, `web/handlers/auth.rs`, `domain/user.rs`, `Cargo.toml`
- **Action Type:** ADD | MODIFY

## 2. MODIFIED_FILES
- `Cargo.toml`: เพิ่ม dependencies `utoipa` (axum_extras, chrono, uuid) และ `utoipa-swagger-ui` (axum)
- `src/domain/user.rs`: เพิ่ม derive `utoipa::ToSchema` และตัวอย่างข้อมูล (`#[schema(example = "...")]`)
- `src/web/handlers/auth.rs`: เพิ่ม attribute macro `#[utoipa::path(...)]` ระบุ HTTP method, path, request_body, response codes, และ security bearer_auth
- `src/web/routes.rs`: ประกาศ Struct `ApiDoc` พร้อม `SecurityAddon` สำหรับ Bearer JWT และ mount Swagger UI เข้ากับ Axum Router

## 3. CONTEXT_AND_REASON
- **Problem/Requirement:** ต้องการหน้าเว็บ UI สำหรับทดสอบ API และเอกสาร OpenAPI Interactive ให้สามารถกดทดสอบสมัครสมาชิก, ล็อกอิน และยิง Protected Endpoints ผ่านเบราว์เซอร์ได้ทันที
- **Previous Behavior:** ไม่มีหน้า Swagger UI ต้องทดสอบผ่าน curl หรือภายนอกเท่านั้น

## 4. IMPLEMENTATION_DETAILS
- **[ADDED]:**
  - ติดตั้ง Swagger UI ที่เส้นทาง `/swagger-ui`
  - ให้บริการ OpenAPI JSON spec ที่เส้นทาง `/api-docs/openapi.json`
  - ปรับปรุง DTOs ทั้งหมดใน `domain/user.rs` ให้มี `ToSchema`
  - กำหนด Security Scheme ชนิด HTTP Bearer (JWT) ทำให้มีปุ่ม `Authorize` บนหน้า Swagger UI
- **[MODIFIED]:**
  - เชื่อมต่อ OpenAPI Spec ของ Handlers ทั้งหมดในหมวด Auth
- **[DEPRECATED/REMOVED]:** ไม่มี

## 5. BREAKING_CHANGES_AND_SIDE_EFFECTS
- **Breaking Changes:** NO
- **Dependencies Added:**
  - `utoipa = { version = "5", features = ["axum_extras", "chrono", "uuid"] }`
  - `utoipa-swagger-ui = { version = "9", features = ["axum"] }`

## 6. EXPECTED_BEHAVIOR
- เมื่อเปิดเซิร์ฟเวอร์ สามารถเข้าเว็บ `http://localhost:3000/swagger-ui` เพื่อดูเอกสารและทดสอบยิง API ทุกเส้นทางได้
- รองรับการนำ Token จาก Login ไปใส่ในปุ่ม `Authorize` เพื่อทดสอบ `/profile`, `/password`, `/me`, `/account` ได้สะดวก

---

# [CHANGE_LOG] 2026-09-05 - Implement OKX Account Linking & AES-256-GCM Encryption

## 1. META_DATA
- **Feature/Issue:** OKX API Key Management, Two-Way AES-256-GCM Encryption, and Account Endpoints
- **Target Component:** `crypto/encryption.rs`, `storage/repositories/account_repository.rs`, `users/account_service.rs`, `web/handlers/account.rs`, `domain/account.rs`, `web/routes.rs`, `web/state.rs`, `config.rs`, `main.rs`, `Cargo.toml`
- **Action Type:** ADD | MODIFY

## 2. MODIFIED_FILES
- `Cargo.toml`: เพิ่ม dependencies `aes-gcm` และ `base64`
- `config/secrets.env`: เพิ่มค่าตั้งค่า `ENCRYPTION_KEY` (Master Key 32 bytes)
- `src/config.rs`: โหลด `encryption_key` เข้าสู่ `AppConfig`
- `src/crypto/encryption.rs`: สร้าง `EncryptionService` พร้อมตรรกะเข้ารหัส/ถอดรหัส AES-256-GCM (สุ่ม 96-bit Nonce ต่อครั้ง)
- `src/crypto/mod.rs`: expose โมดูล `encryption`
- `src/domain/account.rs`: ปรับปรุง `Account` entity, `AccountStatus`, `LinkAccountRequest`, `AccountResponse` (พร้อม API Key masking `xxxx****xxxx` และ Swagger `ToSchema`)
- `src/storage/repositories/account_repository.rs`: สร้าง `AccountRepository` จัดการ MongoDB collection `accounts` (create, find_by_user_id, find_by_id_and_user_id, delete_by_id_and_user_id)
- `src/storage/repositories/mod.rs`: expose โมดูล `account_repository`
- `src/users/account_service.rs`: สร้าง `AccountService` เข้ารหัส Credentials ลับก่อนบันทึก, ตรวจสอบความเป็นเจ้าของบัญชี (Tenant Isolation), และบริการถอดรหัสสำหรับ Bot Engine
- `src/users/mod.rs`: expose โมดูล `account_service`
- `src/web/state.rs`: เพิ่ม `account_service: Arc<AccountService>` ใน `AppState`
- `src/web/handlers/account.rs`: REST API handlers (`link_account`, `list_accounts`, `get_account`, `delete_account`) พร้อม Swagger annotations
- `src/web/handlers/mod.rs`: expose โมดูล `account`
- `src/web/routes.rs`: รวม Router `/api/accounts` (ผูก `require_auth` middleware) และลงทะเบียน OpenAPI Specs
- `src/main.rs`: เริ่มต้น `AccountRepository`, `EncryptionService`, `AccountService` และผูกเข้าสู่ server lifecycle

## 3. CONTEXT_AND_REASON
- **Problem/Requirement:** ต้องการระบบเชื่อมต่อและจัดเก็บ OKX API Credentials (API Key, Secret Key, Passphrase) ของผู้ใช้อย่างปลอดภัยสูงสุด โดยข้อมูลลับต้องถูกเข้ารหัสแบบ 2 ทิศทาง (Two-Way Encryption) เพื่อให้ Bot Engine สามารถถอดรหัสนำไป Sign คำสั่งซื้อขายได้ แต่ภายนอกและใน Database ไม่สามารถอ่านค่า Plaintext ได้
- **Previous Behavior:** ไฟล์ `crypto/encryption.rs`, `account_service.rs`, `account_repository.rs`, และ `handlers/account.rs` ยังเป็นไฟล์ว่างเปล่า

## 4. IMPLEMENTATION_DETAILS
- **[ADDED]:**
  - `EncryptionService`:
    - เข้ารหัสด้วย `Aes256Gcm` (AES-256-GCM Authenticated Encryption)
    - สุ่ม Nonce 96-bit ทุกครั้งที่เข้ารหัส แล้วแพ็ครวมกับ Ciphertext เป็น Base64
    - ถอดรหัสและ Verify Authentication Tag ป้องกันข้อมูลถูกแก้ไข
    - Unit tests ทดสอบทั้งการเข้ารหัส-ถอดรหัสกลับมาตรงกัน และความไม่ซ้ำกันของ Ciphertext แม้ข้อความเดิม
  - `AccountRepository`:
    - เมทอด `create`, `find_by_id`, `find_by_user_id`, `find_by_id_and_user_id`, `delete_by_id_and_user_id` สำหรับ collection `accounts`
  - `AccountService`:
    - `link_account`: Validate input -> Encrypt Secret & Passphrase -> Save -> คืนค่า Masked Response
    - `list_accounts`: คืนค่ารายการบัญชีทั้งหมดของผู้ใช้ที่เรียก
    - `get_account`: คืนค่ารายละเอียดบัญชีเฉพาะของตนเอง
    - `delete_account`: ลบบัญชีเฉพาะของตนเอง
    - `get_decrypted_credentials`: บริการภายในสำหรับ Bot Engine ถอดรหัส Plaintext Credentials
  - Handlers ใน `web/handlers/account.rs`:
    - `POST /api/accounts`: ผูกบัญชีใหม่
    - `GET /api/accounts`: ดึงรายการบัญชีของฉัน
    - `GET /api/accounts/:id`: ดึงข้อมูลบัญชีตาม ID
    - `DELETE /api/accounts/:id`: ยกเลิกการผูกบัญชี
- **[MODIFIED]:**
  - `src/domain/account.rs`: เพิ่ม `mask_api_key` ซ่อนอักขระตรงกลางเพื่อความปลอดภัย
  - `src/web/routes.rs`: นำเข้า schemas และ endpoints เข้า Swagger UI ใน Tag `"Exchange Accounts"`
- **[DEPRECATED/REMOVED]:**
  - นำฟิลด์ `accounts: Vec<Account>` ที่ซ้ำซ้อนออกจาก `User` entity เพื่อแยก Collection `accounts` ชัดเจนตามความสัมพันธ์ 1 User : N Accounts

## 5. BREAKING_CHANGES_AND_SIDE_EFFECTS
- **Breaking Changes:** NO
- **Dependencies Added:**
  - `aes-gcm = "0.10"`
  - `base64 = "0.22"`

## 6. EXPECTED_BEHAVIOR
- ผู้ใช้สามารถเปิด Swagger UI ที่ `http://localhost:3000/swagger-ui` นำ Bearer Token ไปใส่ แล้วทดสอบผูกบัญชี OKX ผ่าน `POST /api/accounts`
- ข้อมูล `secret_key` และ `passphrase` ใน MongoDB Collection `accounts` จะถูกเข้ารหัสเป็น Base64 String ที่ไม่สามารถอ่านค่าได้ตรงๆ
- บอทเทรดสามารถเรียก `account_service.get_decrypted_credentials(account_id, user_id)` เพื่อดึง Key จริงมา Sign Order ได้อย่างปลอดภัย

---

# [CHANGE_LOG] 2026-09-05 - Implement Backend Logout & Token Invalidation via Timestamp

## 1. META_DATA
- **Feature/Issue:** Backend Logout with Server-Side Token Invalidation (`last_logout_at`)
- **Target Component:** `domain/user.rs`, `web/middlewares/auth_middleware.rs`, `web/handlers/auth.rs`, `web/routes.rs`
- **Action Type:** ADD | MODIFY

## 2. MODIFIED_FILES
- `src/domain/user.rs`: เพิ่มฟิลด์ `last_logout_at: Option<DateTime<Utc>>` ใน Struct `User`
- `src/web/middlewares/auth_middleware.rs`: อัปเดต `require_auth` ให้ตรวจเช็ค `claims.iat` เทียบกับ `user.last_logout_at` หาก Token ออกมาก่อนหรือตอนกด Logout จะตัดสิทธิ์ทันที (`401 Unauthorized`)
- `src/web/handlers/auth.rs`: เพิ่ม Handler `POST /api/auth/logout` บันทึกเวลา `last_logout_at = Utc::now()` ลง MongoDB
- `src/web/routes.rs`: ลงทะเบียน Endpoint `POST /api/auth/logout` ภายใต้ `auth_protected_routes` และเพิ่มลงใน OpenAPI Specs (Swagger UI)

## 3. CONTEXT_AND_REASON
- **Problem/Requirement:** ป้องกันช่องโหว่ของ Stateless JWT ที่แม้ผู้ใช้จะสั่งออกจากระบบแล้ว แต่หากมีผู้คัดลอก Token ไว้ Token นั้นจะยังนำมายิง API ได้จนกว่าจะหมดอายุ (24 ชม.) จึงต้องมีกลไก Invalidate Token ฝั่งเซิร์ฟเวอร์
- **Previous Behavior:** ไม่มี Logout API ฝั่ง Backend ต้องพึ่งพาการลบ Token ที่ฝั่ง Client เพียงอย่างเดียว

## 4. IMPLEMENTATION_DETAILS
- **[ADDED]:**
  - ฟิลด์ `last_logout_at` ใน `User` entity
  - Handler `logout`:
    - รับ JWT Token ผ่าน `require_auth`
    - ค้นหา User ใน MongoDB แล้วอัปเดต `last_logout_at = Utc::now()`
    - ส่ง Response ยืนยันการออกจากระบบสำเร็จ
- **[MODIFIED]:**
  - `require_auth` Middleware:
    - ตรวจสอบ `user.is_active()`
    - ตรวจสอบ `claims.iat <= user.last_logout_at.timestamp()` หากเป็นจริงจะส่งกลับ `401 Unauthorized: Token has been revoked. Please log in again.`
  - `src/web/routes.rs`: เชื่อมโยงเส้นทาง `/api/auth/logout` พร้อม OpenAPI Document

## 5. BREAKING_CHANGES_AND_SIDE_EFFECTS
- **Breaking Changes:** NO
- **Dependencies Added:** ไม่มี

## 6. EXPECTED_BEHAVIOR
- เมื่อผู้ใช้เรียก `POST /api/auth/logout` สำเร็จ ระบบจะบันทึก Timestamp ล่าสุด
- Token ปัจจุบันรวมถึง Token เก่าทั้งหมดที่สร้างขึ้นก่อนการ Logout จะไม่สามารถนำกลับมาใช้เรียก Protected Endpoints ได้อีกต่อไป
- หากต้องการใช้งานใหม่ ผู้ใช้ต้องเข้าสู่ระบบผ่าน `POST /api/auth/login` เพื่อรับ Token ที่มีเวลา `iat` ใหม่กว่า `last_logout_at`

---

# [CHANGE_LOG] 2026-09-05 - Implement Pure OKX v5 Connector (Signer & REST Balance Verification)

## 1. META_DATA
- **Feature/Issue:** OKX v5 Authentication Signer, REST Client, and Account Verification Endpoint
- **Target Component:** `okx/signer.rs`, `okx/dto/account.rs`, `okx/rest_client.rs`, `okx/mod.rs`, `users/account_service.rs`, `web/handlers/account.rs`, `web/routes.rs`, `main.rs`, `Cargo.toml`
- **Action Type:** ADD | MODIFY

## 2. MODIFIED_FILES
- `Cargo.toml`: เพิ่ม dependencies `reqwest` (json), `hmac`, `sha2`
- `src/okx/signer.rs`: สร้าง `OkxSigner` สำหรับคำนวณลายเซ็น HMAC-SHA256 Base64 ตามกฎ OKX v5 (`timestamp + method + path + body`) และสร้าง ISO 8601 UTC Timestamp
- `src/okx/dto/account.rs`: นิยาม DTOs สำหรับข้อมูล Balance ของ OKX (`OkxBalanceDetail`, `OkxAccountBalanceData`, `OkxApiResponse`, `AccountVerificationResult`) พร้อม `ToSchema`
- `src/okx/rest_client.rs`: สร้าง `OkxRestClient` ยิง `GET /api/v5/account/balance` พร้อมแนบ Headers (`OK-ACCESS-KEY`, `OK-ACCESS-SIGN`, `OK-ACCESS-TIMESTAMP`, `OK-ACCESS-PASSPHRASE`, `x-simulated-trading`)
- `src/okx/mod.rs`: expose โมดูล `dto`, `rest_client`, `signer`
- `src/users/account_service.rs`: เพิ่มเมทอด `verify_account(account_id, user_id)` ดึง Credentials ที่เข้ารหัสไว้มาถอดรหัสใน Memory แล้วเรียก `OkxRestClient::verify_credentials`
- `src/web/handlers/account.rs`: เพิ่ม Handler `POST /api/accounts/{id}/verify` ทดสอบยิงไปกระดาน OKX จริงและส่งผลลัพธ์ยอดคงเหลือกลับ
- `src/web/routes.rs`: ลงทะเบียน Endpoint `POST /api/accounts/{id}/verify` และลงทะเบียน DTO Schemas ใน OpenAPI / Swagger UI
- `src/main.rs`: expose โมดูล `okx` และเริ่มต้น `OkxRestClient` ส่งเข้า `AccountService`

## 3. CONTEXT_AND_REASON
- **Problem/Requirement:** ต้องการให้ระบบสามารถติดต่อสื่อสารกับ OKX v5 API ได้อย่างถูกต้องด้วยการลงลายเซ็น Cryptographic HMAC-SHA256 และสามารถทดสอบดึง Asset Balance ของบัญชีที่ผูกไว้ เพื่อยืนยันว่า API Key/Secret/Passphrase ถูกต้องและมีสิทธิ์ใช้งานจริง
- **Previous Behavior:** โฟลเดอร์ `src/okx` ยังเป็นไฟล์ว่างเปล่า ระบบยังไม่สามารถติดต่อกับ Exchange ได้

## 4. IMPLEMENTATION_DETAILS
- **[ADDED]:**
  - `OkxSigner`:
    - `generate_timestamp()`: คืนค่า ISO8601 UTC format เช่น `2020-12-08T09:08:57.715Z`
    - `sign()`: คำนวณ HMAC-SHA256 ของ prehash string แล้วแปลงเป็น Base64
    - Unit tests ทดสอบทั้ง timestamp format และลายเซ็น HMAC
  - `OkxRestClient`:
    - `get_balance()`: จัดเตรียม Headers พร้อม Signer แล้วยิง HTTP GET ไปยัง `https://www.okx.com/api/v5/account/balance`
    - `verify_credentials()`: Wrapper สำหรับตรวจสอบสุขภาพของ API Key และดึงยอดคงเหลือ (USDT / BTC / Coins)
  - `AccountVerificationResult`: สรุปผลลัพธ์ `is_valid`, `message`, `total_equity_usd`, `balances`
  - Endpoint `POST /api/accounts/{id}/verify`
- **[MODIFIED]:**
  - `AccountService`: เชื่อมโยง `Arc<OkxRestClient>` เพื่อดึงข้อมูลสำหรับ Verify
  - `src/web/routes.rs`: เพิ่ม OpenAPI Spec ในกลุ่ม `Exchange Accounts`

## 5. BREAKING_CHANGES_AND_SIDE_EFFECTS
- **Breaking Changes:** NO
- **Dependencies Added:**
  - `reqwest = { version = "0.12", features = ["json"] }`
  - `hmac = "0.12"`
  - `sha2 = "0.10"`

## 6. EXPECTED_BEHAVIOR
- ผู้ใช้สามารถเปิด Swagger UI ที่ `http://localhost:3000/swagger-ui`
- เลือก `POST /api/accounts/{id}/verify` ระบุ ID ของ Account ที่ผูกไว้
- ระบบจะถอดรหัส Key ใน Memory และยิงไปเช็คกับ OKX จริง:
  - หาก Key ถูกต้อง: ส่งกลับ `is_valid: true`, ยอด `total_equity_usd` และรายการเหรียญคงเหลือ
  - หาก Key ไม่ถูกต้อง/Passphrase ผิด: ส่งกลับ `is_valid: false` พร้อม Error Message จาก OKX เช่น `50111: Invalid Sign` หรือ `50100: User does not exist`

---

# [CHANGE_LOG] 2026-09-10 - Implement Pure OKX v5 WebSocket Client & Hierarchical Rate Limiter

## 1. META_DATA
- **Feature/Issue:** Pure OKX v5 WebSocket Client (Public Data Feed + Private Trade Dispatcher) and Hierarchical Rate Limiter Engine
- **Target Component:** `okx/rate_limiter.rs`, `okx/ws_client.rs`, `okx/ws_trade.rs`, `okx/manager.rs`, `okx/dto/market.rs`, `okx/dto/trade.rs`, `okx/mod.rs`, `Cargo.toml`
- **Action Type:** ADD | MODIFY

## 2. MODIFIED_FILES
- `Cargo.toml`: เพิ่ม dependencies `tokio-tungstenite = { version = "0.26", features = ["native-tls"] }` และ `tracing = "0.1"`
- `src/okx/rate_limiter.rs`: สร้าง Hierarchical Token Bucket Engine (Sub-account 1,000 req/2s, Single Place 60/2s, Batch Place 300/2s, Cancel/Amend 60/2s, General REST Endpoints) รองรับทั้ง `try_acquire` (non-blocking) และ `acquire` (async wait)
- `src/okx/dto/market.rs`: สร้าง DTOs สำหรับ Public WebSocket Market Data (`WsSubscriptionArg`, `WsRequest`, `WsEventResponse`, `OkxTickerData`, `WsDataMessage`)
- `src/okx/dto/trade.rs`: สร้าง DTOs สำหรับ WebSocket Private Login (`WsLoginArg`), Order Placement (`WsPlaceOrderArg`), และผลลัพธ์ (`WsOrderResult`)
- `src/okx/dto/mod.rs`: expose `market` และ `trade`
- `src/okx/ws_client.rs`: สร้าง `OkxPublicWsClient` จัดการการเชื่อมต่อ Public WebSocket (`wss://ws.okx.com:8443/ws/v5/public`) พร้อม Ping-Pong Heartbeat ทุก 20 วินาที (ป้องกัน 30s idle disconnect), Auto-reconnect with Exponential Backoff (เคารพ limit 3 req/sec), และ Broadcast channel
- `src/okx/ws_trade.rs`: สร้างโครงสร้าง `OkxWsTradeClient` สำหรับยิงคำสั่งซื้อขายผ่าน WebSocket Private API
- `src/okx/manager.rs`: สร้าง `OkxManager` เป็น Connection Pool Controller ถือ Public WS กลาง และจัดการ Registry ของ `OkxRateLimiter` แยกตาม Sub-account
- `src/okx/mod.rs`: expose โมดูล `dto`, `manager`, `rate_limiter`, `rest_client`, `signer`, `ws_client`, `ws_trade`

## 3. CONTEXT_AND_REASON
- **Problem/Requirement:** 
  1. การส่งคำสั่งซื้อขายของบอทจำเป็นต้องควบคุม Rate Limit อย่างเคร่งครัดตามกฎ OKX v5 เพื่อป้องกัน Error 50011 และ Error 50061 (Sub-account order cap)
  2. การดึงราคาตลาดด้วย REST Polling มีความหน่วงสูงและสิ้นเปลืองโควต้า จึงต้องมี Public WebSocket Client รับข้อมูลราคาแบบ Real-time พร้อมกลไก Heartbeat ป้องกันการตัดสาย
  3. สถาปัตยกรรมต้องแยกส่วนชัดเจน: Layer `src/okx` เป็น Pure Exchange Driver ไม่เอาตรรกะของบอทหรือ Order Lifecycle มาปะปน
- **Previous Behavior:** มีเพียง REST Client ดึงยอดคงเหลือ ยังไม่มีตัวจัดการ Rate Limiter และไม่มี WebSocket Client สำหรับรับข้อมูลตลาดสด

## 4. IMPLEMENTATION_DETAILS
- **[ADDED]:**
  - `OkxRateLimiter`:
    - `TokenBucket`: คำนวณ continuous refill ตามเวลา elapsed
    - `try_acquire_order`: ตรวจสอบและหักโควต้า 2 ชั้น (Sub-account หักตามจำนวน N orders จริง, Instrument หัก 1 token สำหรับ Single 60/2s หรือ Batch 300/2s) พร้อม Rollback อัตโนมัติหากชั้นที่สองไม่ผ่าน
    - `acquire_order`: Async loop รอพร้อม timeout ป้องกันการรอค้างเติ่ง
    - `try_acquire_endpoint`: คุมโควต้าของ REST API ทั่วไป
  - `OkxPublicWsClient`:
    - Singleton Hub pattern สำหรับกระจายข้อมูลราคาผ่าน `broadcast::channel`
    - Batch subscription เพื่อไม่ให้เกินโควต้า 480 sub/hr
    - Heartbeat Timer ส่ง raw text `"ping"` ทุก 20 วินาที
    - Reconnection Backoff (1s, 2s, 4s, ..., สูงสุด 30s)
  - `OkxWsTradeClient` & `OkxManager`:
    - จัดเตรียม Helper สร้าง HMAC-SHA256 Signed Login Payload
    - Registry แม็พ Sub-account เข้ากับ `OkxRateLimiter` ประจำตัว
  - Unit tests ใน `rate_limiter.rs` และ Live integration test สตรีมราคา `BTC-USDT` ใน `ws_client.rs`
- **[MODIFIED]:**
  - `src/okx/mod.rs` และ `src/okx/dto/mod.rs`: expose โมดูลใหม่ครบถ้วน

## 5. BREAKING_CHANGES_AND_SIDE_EFFECTS
- **Breaking Changes:** NO
- **Dependencies Added:**
  - `tokio-tungstenite = { version = "0.26", features = ["native-tls"] }`
  - `tracing = "0.1"`

## 6. EXPECTED_BEHAVIOR
- ระบบสามารถทดสอบ Unit Test ผ่าน `cargo test --bin okx-bot-backend` (14 passed)
- สามารถรัน `cargo test --bin okx-bot-backend -- ws_client --nocapture` เพื่อทดสอบต่อ Public WebSocket สตรีมราคาเหรียญ BTC-USDT จริงกลับมาได้สำเร็จ
- การส่งคำสั่งในโมดูลถัดไป (Order Pipeline) จะสามารถขอ Permit จาก `OkxRateLimiter` เพื่อการันตีว่าจะไม่เกิด Error 50011 หรือ 50061

---

# [CHANGE_LOG] 2026-09-15 - Implement Bot Lifecycle Management (Skeleton) & Full OKX v5 WebSocket Suite

## 1. META_DATA
- **Feature/Issue:** Bot Lifecycle Management (CRUD + Start/Stop + Safety Guards) and Complete OKX v5 WebSocket Suite (3 Public Streams, 3 Private Streams, 5 Trading Operations)
- **Target Component:** `domain/strategy.rs`, `domain/strategy/`, `domain/order.rs`, `storage/repositories/strategy_repository.rs`, `bot/manager.rs`, `bot/executor.rs`, `services/strategy_service.rs`, `web/handlers/bot_control.rs`, `web/routes.rs`, `okx/ws_client.rs`, `okx/ws_trade.rs`, `okx/dto/market.rs`, `okx/dto/trade.rs`, `main.rs`, `Cargo.toml`
- **Action Type:** ADD | MODIFY

## 2. MODIFIED_FILES
- `Cargo.toml`: เพิ่ม dependencies `rust_decimal = { version = "1.36", features = ["serde-str"] }` และ `tokio-util = "0.7"` (สำหรับ `CancellationToken`)
- `src/domain/strategy.rs`: 
  - ออกแบบแยก `StrategyConfig` (Static Parameters) ออกจาก `StrategyState` (Runtime Active Orders)
  - `StrategyStatus`: `Created`, `Running`, `Paused`, `Stopped`, `Error`
  - DTOs: `CreateBotRequest`, `UpdateBotRequest`, `BotResponse` พร้อม `utoipa::ToSchema`
- `src/domain/strategy/fixed_ratio_rebalance.rs`: เพิ่ม `ToSchema` ให้ `FixdRatioRebalanceConfig` พร้อม Swagger examples ชัดเจน
- `src/domain/order.rs`: เพิ่ม `Side` และ `TrackedOrder` พร้อม derives และ Swagger schemas
- `src/storage/repositories/strategy_repository.rs`: สร้าง Repository สำหรับ MongoDB collection `strategies` (Create, List, Get, Update Config/Status, Delete)
- `src/bot/executor.rs`: สร้าง `BotExecutor` Background Task loop จัดการ Event stream และรองรับ `CancellationToken` (Graceful Shutdown)
- `src/bot/manager.rs`: สร้าง `BotManager` In-Memory Controller บันทึก `CancellationToken` แยกรายบอท และควบคุม Start/Stop แบบ isolated
- `src/services/strategy_service.rs`: สร้าง `StrategyService` พร้อม Safety Guards:
  - ห้ามแก้ไขหรือลบบอทขณะที่กำลังทำงานอยู่ (`BotIsRunning`)
  - ตรวจสอบความเป็นเจ้าของ Exchange Account ก่อนสร้างบอท
- `src/web/handlers/bot_control.rs`: สร้าง REST API 7 endpoints สำหรับจัดการบอท
- `src/web/routes.rs`: ลงทะเบียน `/api/bots` ภายใต้ Bearer Auth และบันทึก OpenAPI specs
- `src/okx/dto/market.rs`: เพิ่ม `OkxTradeData` (Public trades) และ `OkxOrderBookData` (Order book depths)
- `src/okx/dto/trade.rs`: เพิ่ม Private Streams (`OkxOrderUpdateData`, `OkxMyTradeData`, `OkxAccountBalanceUpdate`) และ 5 Trading Operations (`WsPlaceOrderArg`, `WsAmendOrderArg`, `WsCancelOrderArg`, `WsBatchCancelOrderArg`, `WsOrderResult`)
- `src/okx/ws_client.rs`: อัปเกรด `OkxPublicWsClient` ให้รองรับทั้ง 3 Public Streams (`tickers`, `trades`, `books5`)
- `src/okx/ws_trade.rs`: อัปเกรด `OkxWsTradeClient` ให้รองรับทั้ง 3 Private Streams และ 5 Trading Operations (Create Order, Batch Create, Edit Order, Cancel Order, Batch Cancel)
- `src/main.rs`: เชื่อมต่อ `StrategyRepository`, `BotManager`, `OkxManager`, และ `StrategyService` เข้ากับ `AppState`

## 3. CONTEXT_AND_REASON
- **Problem/Requirement:**
  1. ต้องการระบบจัดการวงจรชีวิตของบอทเทรด (Bot Lifecycle Management Skeleton) ที่รองรับ User Flow บน Web Dashboard: Create -> List -> Edit -> Delete -> Start -> Stop
  2. ต้องมี Safety Guard ป้องกันการแก้พารามิเตอร์หรือลบบอทขณะที่กำลังรัน เพื่อไม่ให้เกิด Orphan Orders บน Exchange
  3. จัดเตรียม WebSocket Infrastructure ให้ครอบคลุมตาม Architecture ของระบบเดิม (Public 3 streams, Private 3 streams, Trading 5 operations)
- **Previous Behavior:** ระบบยังไม่มี Entity สำหรับบอท, ไม่มี API สำหรับควบคุม Start/Stop และ WebSocket ยังรองรับเฉพาะ Ticker

## 4. IMPLEMENTATION_DETAILS
- **[ADDED]:**
  - Bot Lifecycle REST API:
    - `POST /api/bots`: สร้างบอทใหม่
    - `GET /api/bots`: ดึงรายการบอททั้งหมดของ User
    - `GET /api/bots/{id}`: ดูข้อมูลบอทรายตัว
    - `PUT /api/bots/{id}`: แก้ไขบอท (บล็อกเมื่อ Status เป็น Running)
    - `DELETE /api/bots/{id}`: ลบบอท (บล็อกเมื่อ Status เป็น Running)
    - `POST /api/bots/{id}/start`: สั่งรัน Background Task
    - `POST /api/bots/{id}/stop`: สั่งหยุด Background Task
  - Concurrency Runtime:
    - `BotManager`: ถือ `Arc<RwLock<HashMap<String, CancellationToken>>>`
    - `BotExecutor`: Tokio Task วิ่งคู่ขนานแบบแยกอิสระต่อบอท 1 ตัว (Fault Isolation)
  - Full WebSocket Suite:
    - Public: `subscribe_ticker`, `subscribe_trades`, `subscribe_orderbook`
    - Private: `subscribe_orders`, `subscribe_my_trades`, `subscribe_balance`
    - Trading Ops: `create_order_ws`, `create_orders_ws`, `edit_order_ws`, `cancel_order_ws`, `cancel_orders_ws`
- **[MODIFIED]:**
  - `src/domain/strategy.rs`: แยก `Config` ออกจาก `State` ชัดเจน
  - `src/web/state.rs`: เพิ่ม `strategy_service` ใน `AppState`

## 5. BREAKING_CHANGES_AND_SIDE_EFFECTS
- **Breaking Changes:** NO
- **Dependencies Added:**
  - `rust_decimal = { version = "1.36", features = ["serde-str"] }`
  - `tokio-util = "0.7"`

## 6. EXPECTED_BEHAVIOR
- `cargo check` ผ่านสมบูรณ์ (0 errors, 0 warnings)
- ผู้ใช้สามารถเข้า Swagger UI ที่ `http://localhost:3000/swagger-ui` เลือก Tag `"Trading Bots"` เพื่อทดสอบสร้างบอท Fixed Ratio Rebalance, ดูรายการ, และสั่ง Start/Stop ได้ทันที
- การกด Start จะจำลอง Background Loop ใน Memory และการกด Stop จะปิด Task อย่างสง่างามโดยไม่กระทบบอทตัวอื่น
- มีช่องทาง WebSocket ให้พร้อมเชื่อมต่อกับ CEM และ Order Pipeline ใน Phase ถัดไป

---

# [CHANGE_LOG] 2026-09-15 - Migrate Logging to flexi_logger & Graceful Shutdown

## 1. META_DATA
- **Feature/Issue:** Replace `tracing` with `log` + `flexi_logger`, Standardize Bot Log Format, and Graceful Server Shutdown
- **Target Component:** `observability/logging.rs`, `bot/executor.rs`, `okx/ws_client.rs`, `main.rs`
- **Action Type:** ADD | MODIFY

## 2. MODIFIED_FILES
- `src/observability/logging.rs`: สร้างระบบ Logger กลาง (`setup_logger`, `flush_log`) ใช้ `flexi_logger` เขียน log ลงไฟล์ `logs/app_*.log` และแสดงผลใน terminal (stderr) พร้อมกัน
- `src/observability/mod.rs`: expose โมดูล `logging`
- `src/bot/executor.rs`: เปลี่ยนจาก `tracing::*` เป็น `log::*` ทุกจุด และปรับรูปแบบ log ให้ `[Bot #id]` อยู่หน้า emoji เสมอ
- `src/okx/ws_client.rs`: เปลี่ยนจาก `tracing::*` เป็น `log::*` ทุกจุด (8 จุด)
- `src/main.rs`: เพิ่ม `pub mod observability`, เรียก `setup_logger()` ก่อนทุกอย่าง, เปลี่ยน `println!` เป็น `log::info!`, เพิ่ม Graceful Shutdown ด้วย `tokio::signal::ctrl_c()`

## 3. CONTEXT_AND_REASON
- **Problem/Requirement:**
  1. `tracing` macros เป็น no-op เพราะไม่ได้ตั้ง subscriber ใน `main.rs` ทำให้ไม่เห็น log ใน terminal เลย
  2. ต้องการบันทึก log ลงไฟล์ (`logs/`) ควบคู่กับแสดงผลใน terminal ตามรูปแบบ `[timestamp] LEVEL [file:line] message`
  3. ต้องการรูปแบบ Bot log ที่แยกตัวได้ชัดเจน: `[Bot #id]` ต้องอยู่หน้า emoji/status เสมอ
  4. กด `Ctrl+C` แล้ว process exit code ไม่ใช่ 0 (`STATUS_CONTROL_C_EXIT`) ต้องการ graceful shutdown
- **Previous Behavior:** Log จาก `tracing::info!` ไม่แสดงผลเลย, `println!` ไม่มี timestamp/level, กด Ctrl+C ได้ exit code ผิดปกติ

## 4. IMPLEMENTATION_DETAILS
- **[ADDED]:**
  - `src/observability/logging.rs`:
    - `setup_logger()`: ตั้งค่า `flexi_logger` อ่านระดับ log จาก `RUST_LOG` env (fallback เป็น `info`), เขียนลง `logs/app_*.log` + duplicate ไป stderr
    - `flush_log()`: flush buffer ก่อนปิดโปรแกรม กันข้อมูลหาย
    - `log_format()`: Custom format `[YYYY-MM-DD HH:MM:SS.mmm] LEVEL [file:line] message`
  - `src/observability/mod.rs`: โมดูล entry point
  - `shutdown_signal()` ใน `main.rs`: รอรับ `Ctrl+C` แล้ว trigger graceful shutdown ของ Axum server
- **[MODIFIED]:**
  - `src/bot/executor.rs`: `tracing::*` → `log::*` (7 จุด), ย้าย emoji ไว้หลัง `[Bot #id]`
    - ก่อน: `🛑 [Bot #id] Stop requested...`
    - หลัง: `[Bot #id] 🛑 Stop requested...`
  - `src/okx/ws_client.rs`: `tracing::*` → `log::*` (8 จุด)
  - `src/main.rs`: `println!` → `log::info!` (4 จุด), เพิ่ม `with_graceful_shutdown()`
- **[DEPRECATED/REMOVED]:** ไม่มีการใช้ `tracing` macros อีกต่อไปในโค้ด (dependency ยังคงอยู่ใน Cargo.toml เนื่องจาก transitive deps ใช้)

## 5. BREAKING_CHANGES_AND_SIDE_EFFECTS
- **Breaking Changes:** NO
- **Dependencies Added:**
  - `log = "0.4"`
  - `flexi_logger = "0.29"`

## 6. EXPECTED_BEHAVIOR
- เมื่อ `cargo run` จะเห็น log ใน terminal ในรูปแบบ:
  ```
  [2026-09-15 03:30:43.701] INFO [src\main.rs:34] Loading application configuration...
  [2026-09-15 03:30:43.705] INFO [src\main.rs:46] MongoDB connected successfully!
  ```
- Log จะถูกบันทึกลงไฟล์ในโฟลเดอร์ `logs/` พร้อมกัน
- Bot log จะแสดง `[Bot #bot-id]` นำหน้าเสมอ เช่น `[Bot #bot-8f2a1b] 🛑 Cancellation requested...`
- กด `Ctrl+C` จะเห็น log `Ctrl+C received — initiating graceful shutdown...` แล้ว process จะ exit code 0 อย่างเรียบร้อย
