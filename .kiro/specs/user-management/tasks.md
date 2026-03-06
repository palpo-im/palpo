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

- [x] 3.1.1 Write unit tests for all repositories
- [x] 3.1.2 Write unit tests for all handlers
- [x] 3.1.3 Write unit tests for password generator
- [x] 3.1.4 Write unit tests for frontend components
- [x] 3.1.5 Ensure backend tests cover all critical business logic and edge cases (target: ≥70% coverage with high-value tests)
- [x] 3.1.6 Ensure frontend tests cover all user interactions and state changes (target: ≥70% coverage with high-value tests)
- [ ] 3.1.7 Review and remove low-value duplicate tests
- [ ] 3.1.8 Ensure no tests duplicate source file unit tests
- [ ] 3.1.9 Verify all tests provide unique value and focus on business requirements

### 3.2 Property-Based Tests

**Priority: Focus on the most critical properties first (marked with ⭐)**

- [ ] 3.2.1 ⭐ Implement Property 1: Username availability accuracy
- [ ] 3.2.2 ⭐ Implement Property 2: User creation idempotency
- [ ] 3.2.3 ⭐ Implement Property 3: Device deletion token invalidation
- [ ] 3.2.4 ⭐ Implement Property 4: Pagination consistency
- [ ] 3.2.5 Implement Property 5: Sorting stability
- [ ] 3.2.6 ⭐ Implement Property 6: Rate limit config consistency
- [ ] 3.2.7 Implement Property 7: Rate limit deletion emptiness
- [ ] 3.2.8 ⭐ Implement Property 8: Shadow-ban status consistency
- [ ] 3.2.9 Implement Property 9: Threepid lookup accuracy
- [ ] 3.2.10 Implement Property 10: External ID lookup accuracy
- [ ] 3.2.11 Implement Property 11: Search results matching
- [ ] 3.2.12 Implement Property 12: Device list completeness
- [ ] 3.2.13 Implement Property 13: Membership list completeness
- [ ] 3.2.14 ⭐ Implement Property 14: Password reset login
- [ ] 3.2.15 ⭐ Implement Property 15: Admin status setting
- [ ] 3.2.16 Implement Property 16: Media deletion update
- [ ] 3.2.17 ⭐ Implement Property 17: Audit log completeness
- [ ] 3.2.18 ⭐ Implement Property 18: User deactivation state
- [ ] 3.2.19 Implement Property 19: Batch device deletion idempotency
- [ ] 3.2.20 Implement Property 20: Pushers list format consistency

**Note**: Start with ⭐ priority properties. Non-priority properties can be implemented as simple unit tests if PBT overhead is not justified.

### 3.3 Integration Tests

**Focus**: Test multi-component interactions and end-to-end workflows, not individual functions

- [ ] 3.3.1 Test complete user lifecycle (create, modify, deactivate)
- [ ] 3.3.2 Test device management flow
- [ ] 3.3.3 Test rate limit configuration flow
- [ ] 3.3.4 Test media management flow
- [ ] 3.3.5 Test shadow-ban operations
- [ ] 3.3.6 Test permission validation
- [ ] 3.3.7 Test audit logging
- [ ] 3.3.8 Test database operations with PostgreSQL

**Note**: Integration tests should verify that components work together correctly. Avoid duplicating unit test scenarios.

### 3.4 Manual Testing

- [ ] 3.4.1 UI component testing across browsers
- [ ] 3.4.2 Responsive layout testing
- [ ] 3.4.3 Accessibility testing (keyboard navigation, screen reader)
- [ ] 3.4.4 User experience testing
- [ ] 3.4.5 Performance testing with large datasets
- [ ] 3.4.6 Performance comparison with Synapse Admin API

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

**Coverage Philosophy**:
- Target: ≥70% coverage with high-value tests
- Quality > Quantity: 20 meaningful tests > 100 redundant tests
- If coverage is low, ask "what critical behavior is untested?" not "how can I add more tests?"
- Review tests regularly and remove low-value duplicates