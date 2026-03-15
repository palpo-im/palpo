# Palpo Web Configuration Management Tasks

## Overview

This task list tracks the implementation of the Palpo Matrix server web admin interface.

**Main Task**: palpo-web-config (Palpo Web 管理系统)
**Sub-tasks**: 
- default-admin-account (Web UI 管理员认证)
- user-management (用户管理 - 通过 PalpoClient 调用 Palpo HTTP API)

**Reference**: 
- `.kiro/specs/palpo-web-config/design.md` and `.kiro/specs/palpo-web-config/requirements.md`
- `.kiro/specs/default-admin-account/design.md` and `.kiro/specs/default-admin-account/requirements.md`
- `.kiro/specs/user-management/design.md` and `.kiro/specs/user-management/requirements.md`

## Task Status Legend

- [ ] Not started
- [x] Completed
- [-] In progress
- [~] Needs revision (architecture change)

---

## Part A: Server Configuration and Control (palpo-web-config 核心功能)

### A.1 Implement ServerConfigAPI

**Status**: [x]

**Files to create/modify**:
- `crates/admin-server/src/handlers/server_config.rs` - Server configuration handler
- `crates/admin-server/src/server_config.rs` - Server configuration service

**Endpoints to implement**:
- GET /api/v1/config - Get current server configuration
- POST /api/v1/config - Save server configuration
- GET /api/v1/config/validate - Validate configuration
- POST /api/v1/config/reload - Reload configuration without restart

**Implementation details**:
- Read/write TOML configuration file
- Validate configuration parameters
- Support hot-reload of configuration
- Record configuration changes to audit_logs

**Verification**:
```bash
cargo test --package palpo-admin-server server_config -- --nocapture
```

---

### A.2 Implement ServerControlAPI

**Status**: [x]

**Files to create/modify**:
- `crates/admin-server/src/handlers/server_control.rs` - Server control handler
- `crates/admin-server/src/server_control.rs` - Server control service

**Endpoints to implement**:
- GET /api/v1/server/status - Get server status
- POST /api/v1/server/start - Start Palpo server
- POST /api/v1/server/stop - Stop Palpo server
- POST /api/v1/server/restart - Restart Palpo server
- GET /api/v1/server/logs - Get server logs

**Implementation details**:
- Manage Palpo server process (start/stop/restart)
- Monitor server status and health
- Collect server logs
- Record server control operations to audit_logs
- Support graceful shutdown

**Verification**:
```bash
cargo test --package palpo-admin-server server_control -- --nocapture
```

---

### A.3 Implement Server Status Monitoring

**Status**: [ ]

**Files to create/modify**:
- `crates/admin-server/src/handlers/server_status.rs` - Server status handler
- `crates/admin-server/src/server_status.rs` - Server status service

**Endpoints to implement**:
- GET /api/v1/server/health - Server health check
- GET /api/v1/server/metrics - Server metrics (CPU, memory, connections)
- GET /api/v1/server/version - Server version information

**Implementation details**:
- Monitor server health status
- Collect system metrics
- Provide version information
- Support health check for load balancers

**Verification**:
```bash
cargo test --package palpo-admin-server server_status -- --nocapture
```

---

## Part B: Room Management (palpo-web-config 房间管理)

### B.1 Implement Room Management Backend API

**Status**: [ ]

**Files to create/modify**:
- `crates/admin-server/src/handlers/room_handler.rs` - Room management handler
- `crates/admin-server/src/room_service.rs` - Room management service

**Endpoints to implement**:
- GET /api/v1/rooms - List rooms
- GET /api/v1/rooms/:room_id - Get room details
- GET /api/v1/rooms/:room_id/members - Get room members
- GET /api/v1/rooms/:room_id/state - Get room state events
- POST /api/v1/rooms/:room_id/publish - Publish room to directory
- DELETE /api/v1/rooms/:room_id/publish - Unpublish room from directory
- DELETE /api/v1/rooms/:room_id - Delete room

**Implementation details**:
- Query room information from Palpo database
- Support room filtering and pagination
- Support room directory operations
- Record room operations to audit_logs

**Verification**:
```bash
cargo test --package palpo-admin-server room_handler -- --nocapture
```

---

### B.2 Implement Room Management Frontend

**Status**: [ ]

**Files to create/modify**:
- `crates/admin-ui/src/pages/room_manager.rs` - Room management page
- `crates/admin-ui/src/services/room_admin_api.rs` - Room API client

**Components to implement**:
- Room list page with filtering and pagination
- Room detail page with member list
- Room state events viewer
- Room directory management

**Verification**:
```bash
cargo build --package palpo-admin-ui
```

---

## Part C: Media Management (palpo-web-config 媒体管理)

### C.1 Implement Media Management Backend API

**Status**: [ ]

**Files to create/modify**:
- `crates/admin-server/src/handlers/media_handler.rs` - Media management handler
- `crates/admin-server/src/media_service.rs` - Media management service

**Endpoints to implement**:
- GET /api/v1/media/stats - Get media statistics
- GET /api/v1/media - List media files
- DELETE /api/v1/media/:media_id - Delete media file
- POST /api/v1/media/quarantine - Quarantine media
- DELETE /api/v1/media/quarantine - Unquarantine media
- POST /api/v1/media/purge-remote - Purge remote media

**Implementation details**:
- Query media information from Palpo database
- Support media deletion and quarantine
- Support remote media cleanup
- Record media operations to audit_logs

**Verification**:
```bash
cargo test --package palpo-admin-server media_handler -- --nocapture
```

---

### C.2 Implement Media Management Frontend

**Status**: [ ]

**Files to create/modify**:
- `crates/admin-ui/src/pages/media_manager.rs` - Media management page
- `crates/admin-ui/src/services/media_admin_api.rs` - Media API client

**Components to implement**:
- Media statistics page
- Media list page with filtering
- User media statistics page
- Media deletion interface

**Verification**:
```bash
cargo build --package palpo-admin-ui
```

---

## Part D: Federation Management (palpo-web-config 联邦管理)

### D.1 Implement Federation Management Backend API

**Status**: [ ]

**Files to create/modify**:
- `crates/admin-server/src/handlers/federation_handler.rs` - Federation management handler
- `crates/admin-server/src/federation_service.rs` - Federation management service

**Endpoints to implement**:
- GET /api/v1/federation/destinations - List federation destinations
- GET /api/v1/federation/destinations/:destination - Get destination details
- POST /api/v1/federation/destinations/:destination/reset - Reset connection

**Implementation details**:
- Query federation destination information
- Support connection reset
- Record federation operations to audit_logs

**Verification**:
```bash
cargo test --package palpo-admin-server federation_handler -- --nocapture
```

---

### D.2 Implement Federation Management Frontend

**Status**: [ ]

**Files to create/modify**:
- `crates/admin-ui/src/pages/federation_manager.rs` - Federation management page
- `crates/admin-ui/src/services/federation_admin_api.rs` - Federation API client

**Components to implement**:
- Federation destinations list page
- Destination detail page
- Connection status monitoring

**Verification**:
```bash
cargo build --package palpo-admin-ui
```

---

## Part E: Registration Token Management (palpo-web-config 注册令牌管理)

### E.1 Implement Registration Token Backend API

**Status**: [ ]

**Files to create/modify**:
- `crates/admin-server/src/handlers/registration_token_handler.rs` - Registration token handler
- `crates/admin-server/src/registration_token_service.rs` - Registration token service

**Endpoints to implement**:
- GET /api/v1/registration-tokens - List registration tokens
- POST /api/v1/registration-tokens - Create registration token
- PUT /api/v1/registration-tokens/:token - Update registration token
- DELETE /api/v1/registration-tokens/:token - Delete registration token

**Implementation details**:
- Manage registration tokens in Palpo database
- Support token creation with usage limits and expiry
- Record token operations to audit_logs

**Verification**:
```bash
cargo test --package palpo-admin-server registration_token_handler -- --nocapture
```

---

### E.2 Implement Registration Token Frontend

**Status**: [ ]

**Files to create/modify**:
- `crates/admin-ui/src/pages/registration_token_manager.rs` - Registration token page
- `crates/admin-ui/src/services/registration_token_admin_api.rs` - Registration token API client

**Components to implement**:
- Registration token list page
- Token creation form
- Token editing interface

**Verification**:
```bash
cargo build --package palpo-admin-ui
```

---

## Part F: Reports Management (palpo-web-config 举报管理)

### F.1 Implement Reports Management Backend API

**Status**: [ ]

**Files to create/modify**:
- `crates/admin-server/src/handlers/reports_handler.rs` - Reports management handler
- `crates/admin-server/src/reports_service.rs` - Reports management service

**Endpoints to implement**:
- GET /api/v1/reports - List reports
- GET /api/v1/reports/:report_id - Get report details
- DELETE /api/v1/reports/:report_id - Delete report

**Implementation details**:
- Query reports from Palpo database
- Support report filtering and pagination
- Record report operations to audit_logs

**Verification**:
```bash
cargo test --package palpo-admin-server reports_handler -- --nocapture
```

---

### F.2 Implement Reports Management Frontend

**Status**: [ ]

**Files to create/modify**:
- `crates/admin-ui/src/pages/reports_manager.rs` - Reports management page
- `crates/admin-ui/src/services/reports_admin_api.rs` - Reports API client

**Components to implement**:
- Reports list page with filtering
- Report detail page with media preview
- Report deletion interface

**Verification**:
```bash
cargo build --package palpo-admin-ui
```

---

## Part G: Appservice Management (palpo-web-config Appservice管理)

### G.1 Implement Appservice Management Backend API

**Status**: [ ]

**Files to create/modify**:
- `crates/admin-server/src/handlers/appservice_handler.rs` - Appservice management handler
- `crates/admin-server/src/appservice_service.rs` - Appservice management service

**Endpoints to implement**:
- GET /api/v1/appservices - List appservices
- POST /api/v1/appservices - Register appservice
- GET /api/v1/appservices/:app_id - Get appservice details
- DELETE /api/v1/appservices/:app_id - Unregister appservice

**Implementation details**:
- Manage appservice registrations
- Validate YAML configuration
- Record appservice operations to audit_logs

**Verification**:
```bash
cargo test --package palpo-admin-server appservice_handler -- --nocapture
```

---

### G.2 Implement Appservice Management Frontend

**Status**: [ ]

**Files to create/modify**:
- `crates/admin-ui/src/pages/appservice_manager.rs` - Appservice management page
- `crates/admin-ui/src/services/appservice_admin_api.rs` - Appservice API client

**Components to implement**:
- Appservice list page
- Appservice registration form
- YAML configuration editor

**Verification**:
```bash
cargo build --package palpo-admin-ui
```

---

## Part H: Testing and Documentation (palpo-web-config 测试和文档)

### H.1 Write Property-Based Tests

**Status**: [ ]

**File**: `crates/admin-server/tests/property_palpo_web_config.rs`

**Properties to test**:
- Pagination consistency
- Configuration validation
- Server status monitoring

**Verification**:
```bash
cargo test --package palpo-admin-server --test property_palpo_web_config -- --nocapture --ignored
```

---

### H.2 Write Integration Tests

**Status**: [ ]

**File**: `crates/admin-server/tests/integration_palpo_web_config.rs`

**Tests to write**:
- Server configuration lifecycle
- Server control operations
- Room management operations
- Media management operations

**Verification**:
```bash
cargo test --package palpo-admin-server --test integration_palpo_web_config -- --nocapture
```

---

### H.3 Write API Documentation

**Status**: [ ]

**Tasks**:
- Document all API endpoints
- Create request/response examples
- Document error codes and handling

**Verification**:
```bash
# Check documentation completeness
```

---

## Part I: Sub-task Dependencies (子任务依赖检查)

### I.1 Default Admin Account (default-admin-account spec)

**Status**: [ ]

**Reference**: `.kiro/specs/default-admin-account/tasks.md`

**Sub-tasks to complete**:
- [ ] Part A: Web UI Admin Authentication System (A.1-A.5)
- [ ] Part B: Migration System (B.1-B.2)
- [ ] Part C: Audit Logging (C.1)
- [ ] Part D: Frontend Implementation (D.1-D.4)
- [ ] Part E: Testing (E.1-E.4)
- [ ] Part F: Documentation (F.1-F.2)

**Completion criteria**:
- All 21 tasks in default-admin-account spec completed
- Web UI admin authentication system fully functional
- All tests passing

**Verification**:
```bash
# Check default-admin-account spec completion
grep -c "^\- \[x\]" .kiro/specs/default-admin-account/tasks.md
```

---

### I.2 User Management (user-management spec)

**Status**: [ ]

**Reference**: `.kiro/specs/user-management/tasks.md`

**Sub-tasks to complete**:
- [ ] Part A: Architecture Fix Tasks (A.1-A.13)
  - [ ] A.1: Integrate PalpoClient into Module Tree
  - [ ] A.2: Add Missing PalpoClient Methods
  - [ ] A.3-A.9: Rewrite handlers using PalpoClient
  - [ ] A.10: Delete Repository Layer Files
  - [ ] A.11-A.13: Tests and Frontend Update
- [ ] Part B: User Management Frontend Enhancement (B.1-B.6)
- [ ] Part C: Testing and Documentation (C.1-C.3)

**Completion criteria**:
- All 22 tasks in user-management spec completed
- PalpoClient fully integrated
- All user management operations go through PalpoClient
- All tests passing

**Verification**:
```bash
# Check user-management spec completion
grep -c "^\- \[x\]" .kiro/specs/user-management/tasks.md
```

---

## Part J: User Management (palpo-web-config 主线中的用户管理功能)

**NOTE**: User management is a critical feature of palpo-web-config.
Implementation is delegated to the user-management spec, but palpo-web-config tracks completion.

**Key Architecture**:
- All user management operations go through PalpoClient
- PalpoClient calls Palpo `/_synapse/admin/` HTTP API
- admin-server does NOT directly connect to Palpo database
- All operations are recorded to audit_logs

**User Management Features** (implemented via user-management spec):
- User account management (create, modify, deactivate, password reset)
- Device management
- Session management (Whois)
- Membership management
- Rate limit configuration
- Media management (user media)
- Pushers management
- Shadow-ban management
- Third-party identifier lookup

**Frontend Pages** (implemented via user-management spec):
- User list page with filtering and pagination
- User detail page with tabs for devices, connections, pushers, media
- User advanced features page (rooms, memberships, rate limits)
- User account data page (account data, threepids, external IDs)
- Batch user registration page

**Completion Status**:
- [ ] User management architecture fix (Part A of user-management spec)
- [ ] User management frontend enhancement (Part B of user-management spec)
- [ ] User management testing and documentation (Part C of user-management spec)

---

## Part B: Original Tasks (Phase 1 - Foundation - COMPLETED)

### B.1 Project Setup and Infrastructure

- [x] B.1.1 Verify project structure and config files
- [x] B.1.2 Set up development and build toolchain

**Status**: ✅ Completed

---

### B.2 Core Data Models and Error Handling

- [x] B.2.1 Implement configuration data models
- [x] B.2.2 Test configuration data models
- [x] B.2.3 Implement error handling system
- [x] B.2.4 Test error handling system

**Status**: ✅ Completed

---

### B.3 Authentication and Authorization Middleware

- [x] B.3.1 Implement authentication middleware
- [x] B.3.2 Test authentication middleware
- [x] B.3.3 Implement audit logging system
- [x] B.3.4 Test audit logging system

**Status**: ✅ Completed

---

### B.4 Dioxus Frontend Infrastructure

- [x] B.4.1 Set up Dioxus application structure
- [x] B.4.2 Implement API client service
- [x] B.4.3 Test API client service

**Status**: ✅ Completed

---

### B.5 Common UI Components

- [x] B.5.1 Implement common UI components
- [x] B.5.2 Implement layout and navigation components
- [x] B.5.3 Manual test UI components

**Status**: ✅ Completed

---

### B.6 Configuration Management Frontend Pages

- [x] B.6.1 Implement configuration form page
- [x] B.6.2 Implement configuration template page
- [x] B.6.3 Implement configuration import/export page
- [x] B.6.4 Test configuration management functionality

**Status**: ✅ Completed

---

### B.7 User Management Frontend Pages (Framework)

- [x] B.7.1 Implement user list page framework
- [x] B.7.2 Implement user detail page framework

**Status**: ✅ Completed - Framework exists, needs API update (see A.13)

---

### B.8 Room Management Frontend Pages (Framework)

- [x] B.8.1 Implement room list page framework
- [x] B.8.2 Implement room detail page framework

**Status**: ✅ Completed

---

### B.9 Media Management Frontend Pages (Framework)

- [x] B.9.1 Implement media management page framework
- [x] B.9.2 Implement user media statistics page framework

**Status**: ✅ Completed

---

### B.10 Federation Management Frontend Pages (Framework)

- [x] B.10.1 Implement federation destinations page framework

**Status**: ✅ Completed

---

### B.11 Appservice Management Frontend Pages (Framework)

- [x] B.11.1 Implement appservice management page framework

**Status**: ✅ Completed

---

### B.12 Server Control Frontend Pages (Framework)

- [x] B.12.1 Implement server status page framework
- [x] B.12.2 Implement server commands page framework

**Status**: ✅ Completed

---

## Part C: Original Tasks (Phase 2 - User Management - NEEDS REVISION)

### C.1 User Management Backend API (Task Group 14)

**⚠️ WARNING**: Task Group 14 uses direct database connection (wrong architecture).

- [~] C.1.1 Username availability check API
  - **Issue**: Uses UserRepository
  - **Fix**: Rewrite to use PalpoClient (see A.3)
- [~] C.1.2 User lock/unlock functionality
  - **Issue**: Uses UserRepository
  - **Fix**: Rewrite to use PalpoClient (see A.3)
- [~] C.1.3 User suspend functionality (MSC3823)
  - **Issue**: Uses UserRepository
  - **Fix**: Rewrite to use PalpoClient (see A.3)
- [~] C.1.4 Device management API
  - **Issue**: Uses DeviceRepository
  - **Fix**: Rewrite to use PalpoClient (see A.4)
- [~] C.1.5 Connection management API (whois)
  - **Issue**: Uses SessionRepository
  - **Fix**: Rewrite to use PalpoClient (see A.5)
- [~] C.1.6 Pushers management API
  - **Issue**: Uses UserRepository
  - **Fix**: Rewrite to use PalpoClient (see A.3)
- [~] C.1.7 Membership management API
  - **Issue**: Uses UserRepository
  - **Fix**: Rewrite to use PalpoClient (see A.3)
- [~] C.1.8 Rate limit management API
  - **Issue**: Uses RateLimitRepository
  - **Fix**: Rewrite to use PalpoClient (see A.6)
- [~] C.1.9 Experimental features management API
  - **Issue**: Uses UserRepository
  - **Fix**: Rewrite to use PalpoClient (see A.3)
- [~] C.1.10 Account data management API
  - **Issue**: Uses UserRepository
  - **Fix**: Rewrite to use PalpoClient (see A.3)
- [~] C.1.11 Third-party identifier management API
  - **Issue**: Uses ThreepidRepository
  - **Fix**: Rewrite to use PalpoClient (see A.9)
- [~] C.1.12 SSO external ID management API
  - **Issue**: Uses UserRepository
  - **Fix**: Rewrite to use PalpoClient (see A.3)
- [~] C.1.13 Batch user registration API
  - **Issue**: Uses UserRepository
  - **Fix**: Rewrite to use PalpoClient (see A.3)
- [~] C.1.14 User management API property tests
  - **Issue**: Tests UserRepository
  - **Fix**: Rewrite to test PalpoClient (see A.11)
- [~] C.1.15 User management API integration tests
  - **Issue**: Tests UserRepository + Database
  - **Fix**: Rewrite to test PalpoClient (see A.12)
- [~] C.1.16 User management API regression tests
  - **Issue**: Tests UserRepository
  - **Fix**: Rewrite to test PalpoClient

---

### C.2 User Management Frontend Enhancement (Task Group 15)

- [ ] C.2.1 Enhance user list functionality
  - **Status**: Pending - Wait for backend fix
- [ ] C.2.2 Enhance user detail functionality
  - **Status**: Pending - Wait for backend fix
- [ ] C.2.3 Enhance user advanced features
  - **Status**: Pending - Wait for backend fix
- [ ] C.2.4 Enhance user account data features
  - **Status**: Pending - Wait for backend fix
- [ ] C.2.5 Implement batch user registration page
  - **Status**: Pending - Wait for backend fix
- [ ] C.2.6 Test user management frontend
  - **Status**: Pending - Wait for backend fix

---

## Part D: Original Tasks (Phase 3 - Room and Media - PENDING)

### D.1 Room Management Backend API (Task Group 17)

- [ ] D.1.1 Extend RoomAdminAPI
  - Room state events list
  - Room forward extremities query
  - Room directory publish/unpublish
  - Room admin settings
- [ ] D.1.2 Implement room media API
- [ ] D.1.3 Test room management API

**Status**: ⏳ Pending - Depends on architecture fix

---

### D.2 Media Management Backend API (Task Group 18)

- [ ] D.2.1 Extend MediaAdminAPI
  - User media statistics query
  - Media quarantine
  - Media protection
  - Remote media purge
- [ ] D.2.2 Implement user media API
- [ ] D.2.3 Test media management API

**Status**: ⏳ Pending - Depends on architecture fix

---

### D.3 Room Management Frontend Enhancement (Task Group 19)

- [ ] D.3.1 Enhance room list functionality
- [ ] D.3.2 Enhance room detail functionality
- [ ] D.3.3 Test room management frontend

**Status**: ⏳ Pending

---

### D.4 Media Management Frontend Enhancement (Task Group 20)

- [ ] D.4.1 Enhance media management functionality
- [ ] D.4.2 Enhance user media statistics functionality
- [ ] D.4.3 Test media management frontend

**Status**: ⏳ Pending

---

## Part E: Original Tasks (Phase 4 - New Modules - PENDING)

### E.1 Room Directory Management (Task Group 22)

- [ ] E.1.1 Implement room directory backend API
- [ ] E.1.2 Test room directory API
- [ ] E.1.3 Implement room directory frontend
- [ ] E.1.4 Test room directory frontend

**Status**: ⏳ Pending

---

### E.2 Registration Token Management (Task Group 23)

- [ ] E.2.1 Implement registration token backend API
- [ ] E.2.2 Test registration token API
- [ ] E.2.3 Implement registration token frontend
- [ ] E.2.4 Test registration token frontend

**Status**: ⏳ Pending

---

### E.3 Reports Management (Task Group 24)

- [ ] E.3.1 Implement reports management backend API
- [ ] E.3.2 Test reports management API
- [ ] E.3.3 Implement reports management frontend
- [ ] E.3.4 Test reports management frontend

**Status**: ⏳ Pending

---

### E.4 Server Operations (Task Group 25)

- [ ] E.4.1 Implement server operations backend API
- [ ] E.4.2 Test server operations API
- [ ] E.4.3 Implement server operations frontend
- [ ] E.4.4 Test server operations frontend

**Status**: ⏳ Pending

---

### E.5 Custom Menu and User Badges (Task Group 26)

- [ ] E.5.1 Implement custom menu functionality
- [ ] E.5.2 Implement contact support functionality
- [ ] E.5.3 Implement user badges functionality
- [ ] E.5.4 Test custom menu and badges

**Status**: ⏳ Pending

---

## Part F: Testing Tasks

### F.1 Unit Tests

- [x] F.1.1 Unit tests for repository business logic
  - **Issue**: Tests UserRepository
  - **Fix**: Rewrite to test PalpoClient
- [x] F.1.2 Unit tests for handler business logic
  - **Issue**: Tests handlers with UserRepository
  - **Fix**: Rewrite to test handlers with PalpoClient
- [x] F.1.3 Unit tests for password generator
  - **Status**: ✅ Keep
- [x] F.1.4 Unit tests for frontend components
  - **Status**: ✅ Keep
- [x] F.1.5 Security-critical unit tests
  - **Issue**: Tests SQL injection prevention
  - **Fix**: Update for PalpoClient (HTTP injection prevention)
- [x] F.1.6 Review existing tests and remove low-value duplicates
  - **Status**: ✅ Keep
- [x] F.1.7 Verify test quality
  - **Status**: ✅ Keep

---

### F.2 Property-Based Tests

- [x] F.2.1 Username availability accuracy (Property 1)
  - **Issue**: Tests UserRepository
  - **Fix**: Rewrite to test PalpoClient (see A.11)
- [x] F.2.2 Pagination consistency (Property 4)
  - **Issue**: Tests UserRepository
  - **Fix**: Rewrite to test PalpoClient (see A.11)
- [x] F.2.3 Rate limit config round-trip (Property 6)
  - **Issue**: Tests RateLimitRepository
  - **Fix**: Rewrite to test PalpoClient (see A.11)
- [x] F.2.4 Audit log completeness (Property 17)
  - **Status**: ✅ Keep - audit logging unchanged

---

### F.3 Integration Tests

- [x] F.3.1 Complete user lifecycle flow
  - **Issue**: Tests UserRepository + Database
  - **Fix**: Rewrite to test PalpoClient (see A.12)
- [x] F.3.2 Device deletion invalidates tokens
  - **Issue**: Tests DeviceRepository + SessionRepository
  - **Fix**: Rewrite to test PalpoClient
- [x] F.3.3 Password reset enables login
  - **Issue**: Tests UserRepository + Auth service
  - **Fix**: Rewrite to test PalpoClient
- [x] F.3.4 Permission validation across operations
  - **Status**: ✅ Keep - auth middleware unchanged
- [x] F.3.5 Audit logging for all operations
  - **Status**: ✅ Keep - audit logging unchanged
- [x] F.3.6 Transaction rollback on error
  - **Issue**: Tests database transactions
  - **Fix**: Update for audit_logs table only
- [x] F.3.7 Concurrent operations consistency
  - **Issue**: Tests database locking
  - **Fix**: Update for audit_logs table only
- [x] F.3.8 User creation form flow
  - **Status**: ✅ Keep - frontend logic unchanged
- [x] F.3.9 User list search and filter
  - **Status**: ✅ Keep - frontend logic unchanged

---

### F.4 E2E Tests

- [x] F.4.1 Admin creates new user
  - **Status**: ✅ Keep - UI flow unchanged
- [x] F.4.2 Admin manages user devices
  - **Status**: ✅ Keep - UI flow unchanged
- [x] F.4.3 Admin resets user password
  - **Status**: ✅ Keep - UI flow unchanged
- [x] F.4.4 Admin configures rate limits
  - **Status**: ✅ Keep - UI flow unchanged
- [x] F.4.5 Admin searches and filters users
  - **Status**: ✅ Keep - UI flow unchanged

---

### F.5 Manual Testing

- [ ] F.5.1 Cross-browser compatibility
- [ ] F.5.2 Responsive layout on different screen sizes
- [ ] F.5.3 Accessibility with screen readers
- [ ] F.5.4 Keyboard navigation completeness
- [ ] F.5.5 User experience and visual polish
- [ ] F.5.6 Performance with 10,000+ users
- [ ] F.5.7 Performance comparison with Synapse Admin API

**Status**: ⏳ Pending - Do after architecture fix

---

## Part G: Documentation Tasks

### G.1 API Documentation

- [ ] G.1.1 Document all API endpoints
- [ ] G.1.2 Create request/response examples
- [ ] G.1.3 Document error codes and handling
- [ ] G.1.4 Add authentication requirements
- [ ] G.1.5 Document database schema

**Status**: ⏳ Pending

---

### G.2 User Documentation

- [ ] G.2.1 Create user management operation guide
- [ ] G.2.2 Document batch operations
- [ ] G.2.3 Add troubleshooting section
- [ ] G.2.4 Create FAQ document

**Status**: ⏳ Pending

---

### G.3 Development Documentation

- [ ] G.3.1 Update architecture documentation
- [ ] G.3.2 Document database layer design
- [ ] G.3.3 Create testing guide
- [ ] G.3.4 Document deployment procedures

**Status**: ⏳ Pending

---

## Part H: Security and Performance Tasks

### H.1 Security Implementation

- [ ] H.1.1 Implement input validation on all endpoints
- [ ] H.1.2 Add rate limiting for API calls
- [ ] H.1.3 Implement proper error handling without information leakage
- [ ] H.1.4 Add audit logging for all operations
- [ ] H.1.5 Secure sensitive data handling (passwords)
- [ ] H.1.6 Prevent SQL injection with parameterized queries

**Status**: ⏳ Pending

---

### H.2 Performance Optimization

- [ ] H.2.1 Implement caching for user list queries
- [ ] H.2.2 Optimize pagination queries with proper indexing
- [ ] H.2.3 Add connection pooling for database
- [ ] H.2.4 Implement lazy loading for user detail tabs
- [ ] H.2.5 Optimize frontend bundle size
- [ ] H.2.6 Target 2x performance improvement over Synapse Admin API

**Status**: ⏳ Pending

---

## Progress Summary

### Main Task Phases (palpo-web-config 主线)

| Phase | Category | Status |
|-------|----------|--------|
| Phase 1 | Foundation (已完成) | ✅ 12/12 |
| Phase 2 | Server Config & Control | ⏳ 0/3 |
| Phase 3 | Room Management | ⏳ 0/2 |
| Phase 3 | Media Management | ⏳ 0/2 |
| Phase 4 | Federation Management | ⏳ 0/2 |
| Phase 4 | Registration Tokens | ⏳ 0/2 |
| Phase 4 | Reports Management | ⏳ 0/2 |
| Phase 4 | Appservice Management | ⏳ 0/2 |
| Phase 5 | Testing & Documentation | ⏳ 0/3 |

### Sub-task Dependencies (子任务依赖)

| Sub-task | Total | Completed | Status |
|----------|-------|-----------|--------|
| default-admin-account | 21 | 0 | ⏳ Pending |
| user-management | 22 | 0 | ⏳ Pending |

### Overall Progress

**palpo-web-config Main Tasks**: 12/26 (46%)
- Completed: 12 (Foundation phase)
- Pending: 14 (Server config, room, media, federation, tokens, reports, appservice, testing)

**Sub-task Dependencies**: 0/43 (0%)
- default-admin-account: 0/21
- user-management: 0/22

**Total palpo-web-config System**: 12/69 (17%)
- Main tasks: 12/26
- Sub-tasks: 0/43

### Implementation Roadmap

**Phase 1: Foundation** ✅ (Completed)
- Project setup and infrastructure
- Core data models and error handling
- Authentication and authorization middleware
- Dioxus frontend infrastructure
- Common UI components
- Configuration management frontend
- UI page frameworks

**Phase 2: Server Management** ⏳ (Next)
- Server configuration management (A.1)
- Server control (start/stop/restart) (A.2)
- Server status monitoring (A.3)

**Phase 3: Sub-task 1 - Default Admin Account** ⏳ (Parallel)
- Web UI admin authentication system
- Migration system
- Audit logging
- Frontend implementation
- Testing and documentation

**Phase 4: Sub-task 2 - User Management** ⏳ (After Phase 3)
- Architecture fix (integrate PalpoClient)
- User management frontend enhancement
- Testing and documentation

**Phase 5: Room & Media Management** ⏳ (After Phase 2)
- Room management backend and frontend
- Media management backend and frontend

**Phase 6: Federation & Other Features** ⏳ (After Phase 5)
- Federation management
- Registration tokens
- Reports management
- Appservice management

**Phase 7: Testing & Documentation** ⏳ (Final)
- Property-based tests
- Integration tests
- API documentation

---