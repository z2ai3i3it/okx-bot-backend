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
