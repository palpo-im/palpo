# User Management Tasks

## Overview

This task list tracks the implementation of the user management functionality for the Palpo Matrix server web admin interface.

**Architecture**: Admin Server → PalpoClient → `/_synapse/admin/` HTTP API
**Reference**: 
- `.kiro/specs/user-management/design.md` and `.kiro/specs/user-management/requirements.md`
- `.kiro/specs/default-admin-account/design.md` (Web UI 管理员认证)
- `.kiro/specs/palpo-web-config/design.md` (主线任务)

**Key Points**:
- All user management operations go through PalpoClient
- PalpoClient calls Palpo `/_synapse/admin/` HTTP API
- admin-server does NOT directly connect to Palpo database
- admin-server only stores `webui_admins` and `audit_logs` tables
- All operations are recorded to audit_logs

## Task Status Legend

- [ ] Not started
- [x] Completed
- [-] In progress
- [~] Needs revision

---

## Part A: Architecture Fix Tasks (Priority: Urgent)

### A.1 Integrate PalpoClient into Module Tree

**Status**: [ ]

**Files to modify**:
- `crates/admin-server/src/lib.rs` - Add `pub mod palpo_client;`
- `crates/admin-server/src/main.rs` - Initialize PalpoClient from config
- `crates/admin-server/src/lib.rs` - Add PalpoClient to global state

**Steps**:
1. Add module declaration to lib.rs
2. Add PalpoClient to AppState/Depot
3. Initialize PalpoClient in main.rs with credentials from config

**Verification**:
```bash
cargo build --package palpo-admin-server 2>&1 | grep -i "palpo_client"
```

---

### A.2 Add Missing PalpoClient Methods

**Status**: [ ]

**File**: `crates/admin-server/src/palpo_client.rs`

**Methods to add**:
- `get_whois(user_id)` - GET /_synapse/admin/v1/whois/{user_id}
- `list_user_joined_rooms(user_id)` - GET /_synapse/admin/v1/users/{user_id}/joined_rooms
- `get_user_rate_limit(user_id)` - GET /_synapse/admin/v1/users/{user_id}/override_ratelimit
- `set_user_rate_limit(user_id, config)` - POST /_synapse/admin/v1/users/{user_id}/override_ratelimit
- `delete_user_rate_limit(user_id)` - DELETE /_synapse/admin/v1/users/{user_id}/override_ratelimit
- `list_user_media(user_id)` - GET /_synapse/admin/v1/users/{user_id}/media
- `delete_user_media(user_id)` - DELETE /_synapse/admin/v1/users/{user_id}/media
- `list_user_pushers(user_id)` - GET /_synapse/admin/v1/users/{user_id}/pushers
- `shadow_ban_user(user_id)` - POST /_synapse/admin/v1/users/{user_id}/shadow_ban
- `unshadow_ban_user(user_id)` - DELETE /_synapse/admin/v1/users/{user_id}/shadow_ban
- `login_as_user(user_id)` - POST /_synapse/admin/v1/users/{user_id}/login
- `find_user_by_threepid(medium, address)` - GET /_synapse/admin/v1/threepid/{medium}/users/{address}

**Verification**:
```bash
cargo test --package palpo-admin-server palpo_client -- --nocapture
```

---

### A.3 Rewrite user_handler.rs Using PalpoClient

**Status**: [ ]

**File**: `crates/admin-server/src/handlers/user_handler.rs`

**Changes**:
- Remove `UserRepository` dependency
- Add `PalpoClient` from depot
- Call `palpo_client.list_users()`, `get_user()`, `create_or_update_user()`, etc.
- Add `UserResponse::from_palpo_user()` conversion function

**Endpoints to update**:
- GET /api/v1/users - list_users
- GET /api/v1/users/:user_id - get_user
- PUT /api/v1/users/:user_id - create_or_update_user
- GET /api/v1/users/username-available - check_username_availability
- POST /api/v1/users/:user_id/deactivate - deactivate_user
- POST /api/v1/users/:user_id/reset-password - reset_password
- GET/PUT /api/v1/users/:user_id/admin - get_user / set_admin

**Verification**:
```bash
cargo test --package palpo-admin-server user_handler -- --nocapture
```

---

### A.4 Rewrite device_handler.rs Using PalpoClient

**Status**: [ ]

**File**: `crates/admin-server/src/handlers/device_handler.rs`

**Changes**:
- Remove `DeviceRepository` dependency
- Add `PalpoClient` from depot
- Call `palpo_client.list_user_devices()`, `delete_user_device()`, `delete_user_devices()`

**Endpoints to update**:
- GET /api/v1/users/:user_id/devices - list_devices
- DELETE /api/v1/users/:user_id/devices/:device_id - delete_device
- POST /api/v1/users/:user_id/devices/delete - delete_user_devices

**Verification**:
```bash
cargo test --package palpo-admin-server device_handler -- --nocapture
```

---

### A.5 Rewrite session_handler.rs Using PalpoClient

**Status**: [ ]

**File**: `crates/admin-server/src/handlers/session_handler.rs`

**Changes**:
- Remove `SessionRepository` dependency
- Add `PalpoClient` from depot
- Call `palpo_client.get_whois()`

**Endpoints to update**:
- GET /api/v1/users/:user_id/whois - whois

**Verification**:
```bash
cargo test --package palpo-admin-server session_handler -- --nocapture
```

---

### A.6 Rewrite rate_limit_handler.rs Using PalpoClient

**Status**: [ ]

**File**: `crates/admin-server/src/handlers/rate_limit_handler.rs`

**Changes**:
- Remove `RateLimitRepository` dependency
- Add `PalpoClient` from depot
- Call `palpo_client.get/set/delete_user_rate_limit()`

**Endpoints to update**:
- GET /api/v1/users/:user_id/rate-limit - get_rate_limit
- POST /api/v1/users/:user_id/rate-limit - set_rate_limit
- DELETE /api/v1/users/:user_id/rate-limit - delete_rate_limit

**Verification**:
```bash
cargo test --package palpo-admin-server rate_limit_handler -- --nocapture
```

---

### A.7 Rewrite media_handler.rs Using PalpoClient

**Status**: [ ]

**File**: `crates/admin-server/src/handlers/media_handler.rs`

**Changes**:
- Remove `MediaRepository` dependency
- Add `PalpoClient` from depot
- Call `palpo_client.list_user_media()`, `delete_user_media()`

**Endpoints to update**:
- GET /api/v1/users/:user_id/media - list_user_media
- DELETE /api/v1/users/:user_id/media - delete_user_media

**Verification**:
```bash
cargo test --package palpo-admin-server media_handler -- --nocapture
```

---

### A.8 Rewrite shadow_ban_handler.rs Using PalpoClient

**Status**: [ ]

**File**: `crates/admin-server/src/handlers/shadow_ban_handler.rs`

**Changes**:
- Remove `ShadowBanRepository` dependency
- Add `PalpoClient` from depot
- Call `palpo_client.shadow_ban_user()`, `unshadow_ban_user()`

**Endpoints to update**:
- POST /api/v1/users/:user_id/shadow-ban - shadow_ban
- DELETE /api/v1/users/:user_id/shadow-ban - unshadow_ban

**Verification**:
```bash
cargo test --package palpo-admin-server shadow_ban_handler -- --nocapture
```

---

### A.9 Rewrite threepid_handler.rs Using PalpoClient

**Status**: [ ]

**File**: `crates/admin-server/src/handlers/threepid_handler.rs`

**Changes**:
- Remove `ThreepidRepository` dependency
- Add `PalpoClient` from depot
- Call `palpo_client.find_user_by_threepid()`

**Endpoints to update**:
- GET /api/v1/threepid/email/users/:address - find_user_by_email
- GET /api/v1/threepid/msisdn/users/:address - find_user_by_phone

**Verification**:
```bash
cargo test --package palpo-admin-server threepid_handler -- --nocapture
```

---

### A.10 Delete Repository Layer Files

**Status**: [ ]

**Files to delete**:
```bash
rm crates/admin-server/src/user_repository.rs
rm crates/admin-server/src/device_repository.rs
rm crates/admin-server/src/session_repository.rs
rm crates/admin-server/src/rate_limit_repository.rs
rm crates/admin-server/src/media_repository.rs
rm crates/admin-server/src/shadow_ban_repository.rs
rm crates/admin-server/src/threepid_repository.rs
rm crates/admin-server/src/repositories.rs
```

**Files to modify**:
- `crates/admin-server/src/lib.rs` - Remove module declarations
- `crates/admin-server/src/schema.rs` - Keep only `webui_admins` and `audit_logs` tables

**Verification**:
```bash
cargo build --package palpo-admin-server
cargo test --package palpo-admin-server
```

---

### A.11 Write Property-Based Tests (PalpoClient)

**Status**: [ ]

**File**: `crates/admin-server/tests/property_user_palpo_api.rs`

**Properties to test**:
- Rate limit round-trip consistency
- Pagination query consistency
- Username availability accuracy

**Verification**:
```bash
cargo test --package palpo-admin-server --test property_user_palpo_api -- --nocapture --ignored
```

---

### A.12 Write Integration Tests (PalpoClient)

**Status**: [ ]

**File**: `crates/admin-server/tests/integration_user_palpo_api.rs`

**Tests to write**:
- User lifecycle via Palpo API (create → query → deactivate)
- Device management via Palpo API
- Rate limit configuration via Palpo API
- Audit logging for all operations

**Verification**:
```bash
cargo test --package palpo-admin-server --test integration_user_palpo_api -- --nocapture
```

---

### A.13 Update Frontend API Client

**Status**: [ ]

**File**: `crates/admin-ui/src/services/user_admin_api.rs`

**Changes**:
- Update response types to match Palpo API format
- Update field names (e.g., `name` → `user_id`, `admin` → `is_admin`)

**Verification**:
```bash
cargo build --package palpo-admin-ui
```

---

## Part B: User Management Frontend Enhancement (Task Group 15)

### B.1 Enhance User List Functionality

**Status**: [ ]

**File**: `crates/admin-ui/src/pages/user_manager.rs`

**Features to implement**:
- Username availability check (real-time validation)
- Password generator integration
- Batch user operations
- Advanced filtering and search

**Verification**:
```bash
cargo build --package palpo-admin-ui
```

---

### B.2 Enhance User Detail Functionality

**Status**: [ ]

**File**: `crates/admin-ui/src/pages/user_detail.rs`

**Features to implement**:
- Device management tab
- Connection information tab
- Pushers management tab
- Media management tab

**Verification**:
```bash
cargo build --package palpo-admin-ui
```

---

### B.3 Enhance User Advanced Features

**Status**: [ ]

**File**: `crates/admin-ui/src/pages/user_advanced.rs`

**Features to implement**:
- User room membership list
- User membership records
- Rate limit configuration
- Experimental features management

**Verification**:
```bash
cargo build --package palpo-admin-ui
```

---

### B.4 Enhance User Account Data Features

**Status**: [ ]

**File**: `crates/admin-ui/src/pages/user_account_data.rs`

**Features to implement**:
- Account data viewer and editor
- Third-party identifier management
- SSO external ID management

**Verification**:
```bash
cargo build --package palpo-admin-ui
```

---

### B.5 Implement Batch User Registration Page

**Status**: [ ]

**File**: `crates/admin-ui/src/pages/batch_user_registration.rs`

**Features to implement**:
- CSV file upload
- CSV validation and preview
- Batch import with progress tracking
- Import result display

**Verification**:
```bash
cargo build --package palpo-admin-ui
```

---

### B.6 Test User Management Frontend

**Status**: [ ]

**Test files**:
- `crates/admin-ui/tests/user_manager_test.rs`
- `crates/admin-ui/tests/user_detail_test.rs`

**Verification**:
```bash
cargo test --package palpo-admin-ui
```

---

## Part C: Testing and Documentation

### C.1 Write Property-Based Tests

**Status**: [ ]

**File**: `crates/admin-server/tests/property_user_management.rs`

**Properties to test**:
- Username availability accuracy
- Pagination consistency
- Rate limit configuration round-trip
- Audit log completeness

**Verification**:
```bash
cargo test --package palpo-admin-server --test property_user_management -- --nocapture --ignored
```

---

### C.2 Write Integration Tests

**Status**: [ ]

**File**: `crates/admin-server/tests/integration_user_management.rs`

**Tests to write**:
- Complete user lifecycle flow
- Device deletion invalidates tokens
- Password reset enables login
- Permission validation across operations
- Audit logging for all operations

**Verification**:
```bash
cargo test --package palpo-admin-server --test integration_user_management -- --nocapture
```

---

### C.3 Write API Documentation

**Status**: [ ]

**Tasks**:
- Document all user management API endpoints
- Create request/response examples
- Document error codes and handling

**Verification**:
```bash
# Check documentation completeness
```

---

## Progress Summary

### Architecture Fix Tasks (A.1-A.13)
| Task | Status |
|------|--------|
| A.1 Integrate PalpoClient | [ ] |
| A.2 Add missing methods | [ ] |
| A.3 Rewrite user_handler | [ ] |
| A.4 Rewrite device_handler | [ ] |
| A.5 Rewrite session_handler | [ ] |
| A.6 Rewrite rate_limit_handler | [ ] |
| A.7 Rewrite media_handler | [ ] |
| A.8 Rewrite shadow_ban_handler | [ ] |
| A.9 Rewrite threepid_handler | [ ] |
| A.10 Delete repository files | [ ] |
| A.11 Property-based tests | [ ] |
| A.12 Integration tests | [ ] |
| A.13 Update frontend API client | [ ] |

### Frontend Enhancement Tasks (B.1-B.6)
| Task | Status |
|------|--------|
| B.1 Enhance user list | [ ] |
| B.2 Enhance user detail | [ ] |
| B.3 Enhance advanced features | [ ] |
| B.4 Enhance account data | [ ] |
| B.5 Batch registration | [ ] |
| B.6 Frontend testing | [ ] |

### Testing and Documentation (C.1-C.3)
| Task | Status |
|------|--------|
| C.1 Property-based tests | [ ] |
| C.2 Integration tests | [ ] |
| C.3 API documentation | [ ] |

**Total Tasks**: 22
**Completed**: 0 (0%)
**In Progress**: 0 (0%)
**Pending**: 22 (100%)
