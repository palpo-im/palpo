# User Management Tasks

## Overview

This task list tracks the implementation of the user-management feature for the Palpo Matrix server web admin interface. All tasks are derived from the requirements and design documents.

## Task Status Legend

- [ ] Not started
- [x] Completed
- [-] In progress
- [+] Blocked

## Phase 1: Backend Implementation

### 1.1 Database Layer

- [x] 1.1.1 Design database schema for user management tables
- [x] 1.1.2 Create UserRepository trait and implementation
- [x] 1.1.3 Create DeviceRepository trait and implementation
- [x] 1.1.4 Create SessionRepository trait and implementation
- [x] 1.1.5 Create RateLimitRepository trait and implementation
- [x] 1.1.6 Create MediaRepository trait and implementation
- [x] 1.1.7 Create ShadowBanRepository trait and implementation
- [x] 1.1.8 Create ThreepidRepository trait and implementation
- [x] 1.1.9 Implement SQL queries with proper indexing
- [x] 1.1.10 Add connection pooling configuration

### 1.2 API Handlers

- [x] 1.2.1 Create UserHandler with all user management operations
- [x] 1.2.2 Create DeviceHandler with device management operations
- [x] 1.2.3 Create SessionHandler for whois queries
- [x] 1.2.4 Create RateLimitHandler for rate limit configuration
- [x] 1.2.5 Create MediaHandler for media management
- [x] 1.2.6 Create ShadowBanHandler for shadow-ban operations
- [x] 1.2.7 Create ThreepidHandler for third-party identifier lookup
- [x] 1.2.8 Add authentication middleware to all handlers
- [x] 1.2.9 Add audit logging to all operations
- [x] 1.2.10 Implement request validation and error handling

### 1.3 API Endpoints

- [x] 1.3.1 Implement user CRUD endpoints (GET/POST/PUT/DELETE /api/v1/users)
- [x] 1.3.2 Implement user list endpoint with pagination and filtering (GET /api/v1/users)
- [x] 1.3.3 Implement username availability check endpoint (GET /api/v1/users/username-available)
- [x] 1.3.4 Implement user deactivation endpoint (POST /api/v1/users/{user_id}/deactivate)
- [x] 1.3.5 Implement password reset endpoint (POST /api/v1/users/{user_id}/reset-password)
- [x] 1.3.6 Implement admin status endpoints (GET/PUT /api/v1/users/{user_id}/admin)
- [x] 1.3.7 Implement device management endpoints (GET/DELETE /api/v1/users/{user_id}/devices)
- [x] 1.3.8 Implement batch device deletion endpoint (POST /api/v1/users/{user_id}/devices/delete)
- [x] 1.3.9 Implement whois endpoint (GET /api/v1/users/{user_id}/whois)
- [x] 1.3.10 Implement joined rooms endpoint (GET /api/v1/users/{user_id}/joined-rooms)
- [x] 1.3.11 Implement rate limit endpoints (GET/POST/DELETE /api/v1/users/{user_id}/rate-limit)
- [x] 1.3.12 Implement account data endpoint (GET /api/v1/users/{user_id}/account-data)
- [x] 1.3.13 Implement media management endpoints (GET/DELETE /api/v1/users/{user_id}/media)
- [x] 1.3.14 Implement pushers endpoint (GET /api/v1/users/{user_id}/pushers)
- [x] 1.3.15 Implement shadow-ban endpoints (POST/DELETE /api/v1/users/{user_id}/shadow-ban)
- [x] 1.3.16 Implement login as user endpoint (POST /api/v1/users/{user_id}/login)
- [x] 1.3.17 Implement threepid lookup endpoints (GET /api/v1/threepid/{medium}/users/{address})
- [x] 1.3.18 Implement external ID lookup endpoint (GET /api/v1/auth-providers/{provider}/users/{external_id})

### 1.4 Password Generator

- [x] 1.4.1 Implement secure password generation function
- [x] 1.4.2 Add password strength validation
- [x] 1.4.3 Add unit tests for password generator

## Phase 2: Frontend Implementation

### 2.1 API Client Integration

- [x] 2.1.1 Create ApiClient class for our backend API
- [x] 2.1.2 Create API service layer
- [x] 2.1.3 Add authentication token management
- [x] 2.1.4 Implement error handling and retry logic

### 2.2 User List Page

- [x] 2.2.1 Create UsersPage component structure
- [x] 2.2.2 Implement search functionality
- [x] 2.2.3 Implement filtering (admin status, deactivated status)
- [x] 2.2.4 Implement sorting and pagination
- [x] 2.2.5 Add user selection for batch operations
- [x] 2.2.6 Style the user table component

### 2.3 User Detail Page

- [x] 2.3.1 Create UserDetailPage component structure
- [x] 2.3.2 Implement user header with basic info
- [x] 2.3.3 Create tab panel for different sections
- [x] 2.3.4 Implement Overview tab
- [x] 2.3.5 Implement Devices tab with delete functionality
- [x] 2.3.6 Implement Sessions tab
- [x] 2.3.7 Implement Rooms tab (joined rooms)
- [x] 2.3.8 Implement Rate Limit tab with configuration
- [x] 2.3.9 Implement Media tab with delete functionality
- [x] 2.3.10 Implement Pushers tab
- [x] 2.3.11 Implement Security tab (shadow-ban, password reset, admin status)

### 2.4 User Form Component

- [x] 2.4.1 Create UserForm component for create/edit
- [x] 2.4.2 Implement username availability checking
- [x] 2.4.3 Implement password generation
- [x] 2.4.4 Add form validation
- [x] 2.4.5 Style the form components

### 2.5 Action Dialogs

- [x] 2.5.1 Create confirmation dialog for deactivation
- [x] 2.5.2 Create confirmation dialog for device deletion
- [x] 2.5.3 Create confirmation dialog for media deletion
- [x] 2.5.4 Create warning dialog for shadow-ban
- [x] 2.5.5 Create warning dialog for login as user
- [x] 2.5.6 Create password reset dialog

## Phase 3: Testing

### 3.1 Unit Tests

**Focus**: Test critical business logic, not implementation details

- [x] 3.1.1 Write unit tests for repository business logic (NOT simple CRUD operations)
  - Username availability checking logic
  - User search and filtering logic
  - Pagination boundary conditions
  - Data validation and sanitization
- [x] 3.1.2 Write unit tests for handler business logic
  - Permission validation
  - Input validation and error handling
  - State transitions (e.g., user deactivation)
  - Password policy enforcement
- [x] 3.1.3 Write unit tests for password generator
  - Password strength requirements
  - Cryptographic randomness
  - Character set distribution
- [x] 3.1.4 Write unit tests for frontend components
  - User interactions (form submission, button clicks)
  - State management (loading, error states)
  - Validation feedback
- [x] 3.1.5 Security-critical unit tests
  - SQL injection prevention (parameterized queries)
  - XSS prevention in user inputs
  - Authentication token validation
  - Authorization checks
- [x] 3.1.6 Review existing tests and remove low-value duplicates
  - Remove tests for trivial getters/setters
  - Remove tests for Debug/Display traits
  - Remove tests that duplicate source file tests
  - Consolidate similar test scenarios
- [x] 3.1.7 Verify test quality (NOT coverage percentage)
  - Each test has clear business justification
  - Tests focus on "what" not "how"
  - Tests catch real bugs, not just exercise code

**Quality Review**: Focus on test value, not coverage percentage
- Review tests for business justification
- Ensure tests catch real bugs
- Remove redundant or low-value tests

### 3.2 Property-Based Tests

**Philosophy**: Use PBT only where it provides unique value over unit tests

**Critical Properties (Implement as PBT)**:
- [x] 3.2.1 ⭐ Username availability accuracy (Property 1)
  - Justification: Tests invariant across all possible usernames
  - Value: Catches edge cases in username validation
- [x] 3.2.2 ⭐ Pagination consistency (Property 4)
  - Justification: Tests invariant across all page sizes and offsets
  - Value: Catches off-by-one errors and boundary conditions
- [x] 3.2.3 ⭐ Rate limit config round-trip (Property 6)
  - Justification: Tests invariant across all valid config values
  - Value: Catches serialization/deserialization bugs
- [x] 3.2.4 ⭐ Audit log completeness (Property 17)
  - Justification: Tests invariant across all operations
  - Value: Critical for security compliance

**Properties to Skip** (Already covered by other tests):
- ~~Property 2: User creation idempotency~~ → Covered by 3.3.1
- ~~Property 3: Device deletion token invalidation~~ → Covered by 3.3.2
- ~~Property 5: Sorting stability~~ → Covered by existing unit tests
- ~~Property 7: Rate limit deletion~~ → Covered by unit tests
- ~~Property 8: Shadow-ban status consistency~~ → Covered by 3.3.5
- ~~Property 9-13: Lookup/list operations~~ → Covered by integration tests
- ~~Property 14: Password reset login~~ → Covered by 3.3.3
- ~~Property 15: Admin status setting~~ → Covered by 3.3.4
- ~~Property 16: Media deletion~~ → Covered by integration tests
- ~~Property 18: User deactivation state~~ → Covered by 3.3.1
- ~~Property 19-20: Idempotency/format~~ → Covered by unit tests

**Decision Criteria for PBT**:
- Use PBT when: Testing mathematical properties, invariants across infinite input space, complex state machines
- Use unit tests when: Testing specific scenarios, edge cases, error conditions
- Use integration tests when: Testing multi-component interactions

**Total PBT Count**: 4 critical properties (down from 20)

### 3.3 Integration Tests

**Focus**: Test multi-component interactions, NOT individual functions

**API Integration Tests** (Backend):
- [x] 3.3.1 Complete user lifecycle flow
  - Create user → Verify in DB → Modify user → Verify changes → Deactivate → Verify state
  - Tests: Repository + Handler + Database interaction
- [x] 3.3.2 Device deletion invalidates tokens
  - Create device → Get token → Delete device → Verify token invalid
  - Tests: DeviceRepository + SessionRepository + Auth middleware
- [x] 3.3.3 Password reset enables login
  - Reset password → Attempt login with new password → Verify success
  - Tests: UserRepository + Auth service integration
- [x] 3.3.4 Permission validation across operations
  - Non-admin attempts admin operation → Verify 403
  - Admin performs operation → Verify success
  - Tests: Auth middleware + All handlers
- [x] 3.3.5 Audit logging for all operations
  - Perform operation → Verify audit log entry created
  - Tests: All handlers + Audit logger integration

**Database Integration Tests**:
- [ ] 3.3.6 Transaction rollback on error
  - Start transaction → Cause error → Verify rollback
  - Tests: Repository error handling + Database transactions
- [ ] 3.3.7 Concurrent operations consistency
  - Multiple concurrent user creations → Verify no duplicates
  - Tests: Repository + Database locking

**Frontend Integration Tests** (Component + API):
- [ ] 3.3.8 User creation form flow
  - Fill form → Check username availability → Submit → Verify API call → Verify UI update
  - Tests: UserForm + ApiClient + State management
- [ ] 3.3.9 User list search and filter
  - Enter search → Apply filters → Verify API call → Verify results displayed
  - Tests: UsersPage + ApiClient + Pagination

**What NOT to test here**:
- ❌ Individual repository methods (unit tests)
- ❌ Individual handler methods (unit tests)
- ❌ UI component rendering (unit tests)
- ❌ Simple CRUD operations (covered by E2E tests)

### 3.4 E2E Tests (Automated)

**Required for Web Projects**: Use Playwright/Cypress for real user workflows

**Critical User Journeys**:
- [ ] 3.4.1 Admin creates new user
  - Navigate to users page → Click "Create User" → Fill form → Check username availability → Generate password → Submit → Verify user appears in list
  - Validates: Requirements 1, 2, 3, 19
- [ ] 3.4.2 Admin manages user devices
  - Navigate to user detail → Click Devices tab → View device list → Delete device → Confirm → Verify device removed
  - Validates: Requirements 8
- [ ] 3.4.3 Admin resets user password
  - Navigate to user detail → Click Security tab → Click "Reset Password" → Enter new password → Submit → Verify success message
  - Validates: Requirements 6
- [ ] 3.4.4 Admin configures rate limits
  - Navigate to user detail → Click Rate Limit tab → Set limits → Save → Verify saved → Reset limits → Verify cleared
  - Validates: Requirements 11
- [ ] 3.4.5 Admin searches and filters users
  - Navigate to users page → Enter search term → Apply admin filter → Apply deactivated filter → Sort by name → Verify results
  - Validates: Requirements 19

**E2E Test Requirements**:
- ✅ Must use real browser (Playwright/Cypress)
- ✅ Must simulate real user actions (no API shortcuts)
- ✅ Must verify visual feedback (loading states, error messages)
- ✅ Must verify URL changes and navigation
- ✅ Must check for console errors
- ❌ DO NOT bypass UI with direct API calls
- ❌ DO NOT skip visual verification

### 3.5 Manual Testing

**Focus**: Things that can't be automated

- [ ] 3.5.1 Cross-browser compatibility (Chrome, Firefox, Safari, Edge)
- [ ] 3.5.2 Responsive layout on different screen sizes
- [ ] 3.5.3 Accessibility with screen readers (NVDA, JAWS, VoiceOver)
- [ ] 3.5.4 Keyboard navigation completeness
- [ ] 3.5.5 User experience and visual polish
- [ ] 3.5.6 Performance with 10,000+ users
- [ ] 3.5.7 Performance comparison with Synapse Admin API (target: 2x faster)

## Phase 4: Documentation

### 4.1 API Documentation

- [ ] 4.1.1 Document all API endpoints
- [ ] 4.1.2 Create request/response examples
- [ ] 4.1.3 Document error codes and handling
- [ ] 4.1.4 Add authentication requirements
- [ ] 4.1.5 Document database schema

### 4.2 User Documentation

- [ ] 4.2.1 Create user management operation guide
- [ ] 4.2.2 Document batch operations
- [ ] 4.2.3 Add troubleshooting section
- [ ] 4.2.4 Create FAQ document

### 4.3 Development Documentation

- [ ] 4.3.1 Update architecture documentation
- [ ] 4.3.2 Document database layer design
- [ ] 4.3.3 Create testing guide
- [ ] 4.3.4 Document deployment procedures

## Phase 5: Security and Performance

### 5.1 Security Implementation

- [ ] 5.1.1 Implement input validation on all endpoints
- [ ] 5.1.2 Add rate limiting for API calls
- [ ] 5.1.3 Implement proper error handling without information leakage
- [ ] 5.1.4 Add audit logging for all operations
- [ ] 5.1.5 Secure sensitive data handling (passwords)
- [ ] 5.1.6 Prevent SQL injection with parameterized queries

### 5.2 Performance Optimization

- [ ] 5.2.1 Implement caching for user list queries
- [ ] 5.2.2 Optimize pagination queries with proper indexing
- [ ] 5.2.3 Add connection pooling for database
- [ ] 5.2.4 Implement lazy loading for user detail tabs
- [ ] 5.2.5 Optimize frontend bundle size
- [ ] 5.2.6 Target 2x performance improvement over Synapse Admin API

## Dependencies

### External Dependencies

- PostgreSQL 15+
- Rust 1.70+
- Actix-web for API server
- SQLx for database operations
- proptest for property-based testing
- React 18+
- TypeScript 5+

### Internal Dependencies

- palpo-web-config: API client infrastructure
- default-admin-account: Authentication system
- audit-logging: Audit logging infrastructure

## Progress Tracking

### Sprint 1: Database Foundation
- [ ] Complete database schema design
- [ ] Complete all repository implementations
- [ ] Complete password generator
- [ ] Begin backend unit tests

### Sprint 2: Backend API
- [ ] Complete all API handlers
- [ ] Complete all API endpoints
- [ ] Complete authentication middleware
- [ ] Begin backend unit tests

### Sprint 3: Frontend Foundation
- [ ] Complete API client integration
- [ ] Complete user list page
- [ ] Complete user detail page structure
- [ ] Complete user form component

### Sprint 4: Feature Completion
- [ ] Complete all detail page tabs
- [ ] Complete all action dialogs
- [ ] Complete backend unit tests
- [ ] Begin property-based tests

### Sprint 5: Testing and Polish
- [ ] Complete property-based tests
- [ ] Complete integration tests
- [ ] Complete manual testing
- [ ] Complete documentation

### Sprint 6: Security and Performance
- [ ] Complete security implementation
- [ ] Complete performance optimization
- [ ] Final testing and bug fixes
- [ ] Release preparation

## Notes

- All database operations should use parameterized queries to prevent SQL injection
- Error messages should be user-friendly but not expose sensitive information
- Audit logs should capture all administrative actions
- Performance should be monitored and optimized continuously
- Target 2x performance improvement over Synapse Admin API
- Security reviews should be conducted before release
- Database indexes should be optimized for common query patterns

### Test Quality Guidelines

**CRITICAL**: Focus on test quality over coverage numbers
- ❌ DO NOT add tests just to increase coverage percentage
- ❌ DO NOT duplicate tests that already exist in source files
- ❌ DO NOT test implementation details (Debug traits, display formatting)
- ✅ DO test critical business logic and edge cases
- ✅ DO test integration points between components
- ✅ DO test security-critical functionality (authentication, authorization, validation)
- ✅ DO ensure each test provides unique value

**Test Quality Philosophy**:
- Quality > Quantity: 20 meaningful tests > 100 redundant tests
- Coverage is a metric for review, NOT a goal
- When reviewing coverage, ask: "What critical behavior is untested?" not "How can I add more tests?"
- Regularly review and remove low-value tests
- High coverage with meaningless tests = false confidence
- Low coverage with high-value tests = real confidence