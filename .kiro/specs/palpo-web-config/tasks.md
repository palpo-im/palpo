# Palpo Web Admin Interface - MVP Implementation Tasks

## Overview

This task list tracks the implementation of the Palpo Matrix server web admin interface MVP (Minimum Viable Product).

**Scope**: This spec covers 8 core requirements (MVP):
1. User Management (Req 1 + Req 12)
2. Room Management (Req 2)
3. Media Management (Req 3)
4. Registration Token Management (Req 6)
5. Authentication & Authorization (Req 9)
6. UI/UX (Req 10)
7. Configuration Management (Req 11)
8. Server Control (Req 13 - already completed)

**Priority**: Configuration Management is the highest priority - must be completed before server startup and monitoring.

**Task Status Legend**:
- [ ] Not started
- [x] Completed
- [-] In progress
- [~] Needs revision

---

## Part A: Configuration Management (优先级: 最高 - 必须先完成)

### A.1 Implement Backend Configuration API

**Status**: [x] **COMPLETED**

**Description**: 实现后端配置管理 API，支持表单编辑、TOML编辑、验证和导入/导出

**Files to create/modify**:
- [ ] `crates/admin-server/src/server_config.rs` - Add configuration management methods
- [ ] `crates/admin-server/src/handlers/server_config.rs` - Add configuration endpoints

**Endpoints to implement**:

**表单编辑模式 API:**
- [ ] GET /api/v1/config/form - Get parsed configuration as form data
- [ ] POST /api/v1/config/form - Save configuration from form data
- [ ] GET /api/v1/config/metadata - Get configuration metadata (field descriptions, defaults, validation rules)
- [ ] POST /api/v1/config/reset - Reset configuration to last saved state
- [ ] POST /api/v1/config/reload - Reload configuration from file (without restart)
- [ ] GET /api/v1/server/version - Get server version information
- [ ] GET /api/v1/config/search - Search configuration items by label/description

**TOML编辑模式 API:**
- [ ] GET /api/v1/config/toml - Get raw TOML file content
- [ ] POST /api/v1/config/toml - Save raw TOML file content
- [ ] POST /api/v1/config/toml/validate - Validate TOML syntax and content
- [ ] POST /api/v1/config/toml/parse - Parse TOML and return as JSON

**导入/导出 API:**
- [ ] POST /api/v1/config/export - Export configuration (JSON/YAML/TOML)
- [ ] POST /api/v1/config/import - Import and validate configuration (JSON/YAML/TOML)

**Implementation details**:
- [ ] Define configuration metadata structure (field descriptions, defaults, validation rules)
- [ ] Implement form data parser (convert between form data and TOML)
- [ ] Read TOML file as raw text
- [ ] Validate TOML syntax (using toml crate)
- [ ] Validate TOML content (required fields, types, ranges)
- [ ] Convert TOML to JSON for frontend display
- [ ] Convert JSON to TOML for saving
- [ ] Handle TOML parsing errors with line/column information
- [ ] Support JSON/YAML/TOML import/export formats
- [ ] Record configuration changes to audit_logs
- [ ] Implement configuration search functionality

**Tests**:
- [ ] Test form data parsing and validation
- [ ] Test TOML syntax validation
- [ ] Test TOML content validation
- [ ] Test TOML to JSON conversion
- [ ] Test JSON to TOML conversion
- [ ] Test error handling with line/column info
- [ ] Test import/export for JSON/YAML/TOML formats
- [ ] Test configuration search functionality

---

### A.2 Implement Frontend TOML Editor

**Status**: [x] **COMPLETED**

**Description**: 实现前端 TOML 编辑器，允许用户直接编辑 TOML 文件

**Files to create/modify**:
- [ ] `crates/admin-ui/src/pages/config_toml_editor.rs` - TOML editor page
- [ ] `crates/admin-ui/src/components/toml_editor.rs` - TOML code editor component
- [ ] `crates/admin-ui/src/services/config_api.rs` - Config API client

**Components to implement**:
- [ ] TomlEditor - Main TOML editor component
- [ ] CodeEditor - Code editor with syntax highlighting
- [ ] TomlValidationError - TOML validation error display with line/column info

**Features to implement**:
- [ ] Load raw TOML file content from backend (GET /api/v1/config/toml)
- [ ] Display TOML with syntax highlighting
- [ ] Show line numbers
- [ ] Support undo/redo functionality
- [ ] Real-time TOML syntax validation
- [ ] Validate TOML content (required fields, types, ranges)
- [ ] Show validation errors with line/column information
- [ ] Save/Reset buttons
- [ ] Dirty state tracking
- [ ] Support Ctrl+S keyboard shortcut for save

**Libraries to use**:
- [ ] `syntect` or `highlight.rs` for syntax highlighting
- [ ] `toml` crate for TOML parsing and validation

**Tests**:
- [ ] Test TOML loading and display
- [ ] Test syntax highlighting
- [ ] Test TOML validation
- [ ] Test error display with line/column info
- [ ] Test save/reset functionality
- [ ] Test Ctrl+S keyboard shortcut

---

### A.3 Implement Frontend Configuration Form Editor

**Status**: [x] **COMPLETED**

**Description**: 实现前端表单编辑模式，允许用户通过友好的表单界面编辑配置

**Files to create/modify**:
- [ ] `crates/admin-ui/src/pages/config_form_editor.rs` - Form-based config editor
- [ ] `crates/admin-ui/src/components/config_form_fields.rs` - Config form field components
- [ ] `crates/admin-ui/src/components/config_header.rs` - Config page header with version and reload button

**Components to implement**:
- [ ] ConfigFormEditor - Main form editor component
- [ ] ServerConfigForm - Server configuration form
- [ ] DatabaseConfigForm - Database configuration form
- [ ] FederationConfigForm - Federation configuration form
- [ ] AuthConfigForm - Authentication configuration form
- [ ] MediaConfigForm - Media configuration form
- [ ] NetworkConfigForm - Network configuration form
- [ ] LoggingConfigForm - Logging configuration form
- [ ] ConfigHeader - Header with version info and reload button
- [ ] ConfigSearch - Search/filter configuration fields

**Features to implement**:
- [ ] Load current configuration from backend
- [ ] Display configuration in organized sections (7 categories)
- [ ] Real-time field validation with error messages
- [ ] Dirty state tracking (mark unsaved changes)
- [ ] Save/Reset buttons (disabled when no changes)
- [ ] Validate button to check configuration before saving
- [ ] Search/filter configuration fields (fuzzy search by label and description)
- [ ] Display field descriptions and default values
- [ ] Support undo individual field changes
- [ ] Reload configuration button (refresh from server without restart)
- [ ] Display server version information in header

**Tests**:
- [ ] Test form loading and display
- [ ] Test field validation
- [ ] Test dirty state tracking
- [ ] Test save/reset functionality
- [ ] Test search/filter functionality
- [ ] Test reload configuration
- [ ] Test version display

---

### A.4 Implement Configuration Mode Switching

**Status**: [ ] **NOT STARTED - 优先级: 高**

**Description**: 实现表单编辑和 TOML 编辑模式之间的切换

**Files to create/modify**:
- [ ] `crates/admin-ui/src/pages/config_manager.rs` - Main config manager with mode switching
- [ ] `crates/admin-ui/src/components/config_mode_tabs.rs` - Mode selection tabs
- [ ] `crates/admin-ui/src/components/unsaved_changes_dialog.rs` - Unsaved changes confirmation dialog

**Features to implement**:
- [ ] Tab navigation between Form Edit and TOML Edit modes
- [ ] Detect unsaved changes when switching modes
- [ ] Show confirmation dialog with three options: "Save", "Discard", "Continue Editing"
- [ ] Sync form data and TOML content (convert between formats)

**Tests**:
- [ ] Test mode switching without changes
- [ ] Test mode switching with unsaved changes
- [ ] Test all three dialog options (Save, Discard, Continue)
- [ ] Test form-to-TOML conversion
- [ ] Test TOML-to-form conversion

---

### A.5 Implement Configuration Validation Before Server Start

**Status**: [ ] **NOT STARTED - 优先级: 高**

**Description**: 在启动 Palpo 服务器前验证配置

**Files to create/modify**:
- [ ] `crates/admin-ui/src/pages/server_control.rs` - Extend with pre-start validation
- [ ] `crates/admin-ui/src/components/config_summary.rs` - Config summary display
- [ ] `crates/admin-ui/src/components/server_startup_dialog.rs` - Server startup confirmation dialog

**Features to implement**:
- [ ] Show configuration summary before start (key config items)
- [ ] Call config validation API before allowing start
- [ ] Display validation result: "配置有效" or "配置无效"
- [ ] If invalid, show error details and prevent start
- [ ] If valid, show "配置已验证" and allow start
- [ ] After successful start, show "服务器已启动" success message

**Tests**:
- [ ] Test config summary display
- [ ] Test validation API call
- [ ] Test valid configuration flow
- [ ] Test invalid configuration flow
- [ ] Test success message display

---

### A.6 Implement Configuration Import/Export

**Status**: [ ] **NOT STARTED - 优先级: 中**

**Description**: 实现配置导入/导出功能

**Files to create/modify**:
- [ ] `crates/admin-ui/src/pages/config_import_export.rs` - Import/export page

**Features to implement**:
- [ ] Export current configuration as JSON/YAML/TOML
- [ ] Import configuration from file (JSON/YAML/TOML)
- [ ] Validate imported configuration before applying
- [ ] Show preview of imported configuration

**Tests**:
- [ ] Test export to JSON/YAML/TOML
- [ ] Test import from JSON/YAML/TOML
- [ ] Test validation of imported config

---

## Part B: Server Control (已完成 - 可用于启动和监控)

### B.1 Implement ServerConfigAPI

**Status**: [x] **COMPLETED**

**Files implemented**:
- ✅ `crates/admin-server/src/handlers/server_config.rs`
- ✅ `crates/admin-server/src/server_config.rs`

**Endpoints implemented**:
- ✅ GET /api/v1/config - Get current server configuration
- ✅ POST /api/v1/config - Save server configuration
- ✅ GET /api/v1/config/validate - Validate configuration
- ✅ POST /api/v1/config/reload - Reload configuration without restart

---

### B.2 Implement ServerControlAPI

**Status**: [x] **COMPLETED**

**Files implemented**:
- ✅ `crates/admin-server/src/handlers/server_control.rs`
- ✅ `crates/admin-server/src/server_control.rs`

**Endpoints implemented**:
- ✅ GET /api/v1/server/status - Get server status
- ✅ POST /api/v1/server/start - Start Palpo server
- ✅ POST /api/v1/server/stop - Stop Palpo server
- ✅ POST /api/v1/server/restart - Restart Palpo server

---

### B.3 Implement Server Status Monitoring

**Status**: [x] **COMPLETED**

**Files implemented**:
- ✅ `crates/admin-server/src/handlers/server_status.rs`
- ✅ `crates/admin-server/src/server_status.rs`

**Endpoints implemented**:
- ✅ GET /api/v1/server/health - Server health check
- ✅ GET /api/v1/server/metrics - Server metrics
- ✅ GET /api/v1/server/version - Server version information

---

## Part C: User Management (优先级: 高 - 需要重做架构)

### C.1 Fix User Management Architecture

**Status**: [~] **NEEDS REVISION**

**Description**: 用户管理需要重做架构，改为调用 PalpoClient HTTP API

**Reference**: `.kiro/specs/user-management/design.md`

---

## Part D: Room Management (优先级: 中 - 待开发)

### D.1 Implement Room Management Backend API

**Status**: [ ] **NOT STARTED**

---

## Part E: Media Management (优先级: 中 - 待开发)

### E.1 Implement Media Management Backend API

**Status**: [ ] **NOT STARTED**

---

## Part F: Registration Token Management (优先级: 中 - 待开发)

### F.1 Implement Registration Token Management Backend API

**Status**: [ ] **NOT STARTED**

---

## Implementation Order

**Phase 1 (必须完成 - 启动前提)**:
1. A.1 - Backend Configuration API
2. A.2 - Frontend TOML Editor
3. A.3 - Frontend Configuration Form Editor
4. A.4 - Configuration Mode Switching
5. A.5 - Configuration Validation Before Server Start

**Phase 2 (可选 - 增强功能)**:
6. A.6 - Configuration Import/Export

**Phase 3 (后续 - 其他功能)**:
7. C.1 - Fix User Management Architecture
8. D.1 - Room Management
9. E.1 - Media Management
10. F.1 - Registration Token Management
