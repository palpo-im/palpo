# Default Admin Account API Documentation

## Overview

This document provides comprehensive API documentation for the Default Admin Account system in Palpo Matrix Server Web Admin Interface. The API handles Web UI administrator authentication, password management, and session management.

**Base URL**: `http://localhost:8080/api/v1`

**Authentication**: Bearer token in `Authorization` header (except for `/auth/setup` and `/auth/login`)

---

## Authentication Endpoints

### 1. Check Initialization Status

Check whether the initial admin password has been set up.

**Endpoint**: `GET /auth/status`

**Authentication**: Not required

**Request**:
```bash
curl -X GET http://localhost:8080/api/v1/auth/status
```

**Response (Not Initialized)**:
```json
{
  "initialized": false,
  "message": "Admin account not yet configured. Please run setup wizard."
}
```

**Response (Initialized)**:
```json
{
  "initialized": true,
  "message": "Admin account is configured."
}
```

**Status Codes**:
- `200 OK` - Status retrieved successfully
- `500 Internal Server Error` - Database error

---

### 2. Setup Initial Password

Set up the initial admin password. This endpoint is only available when the system is not yet initialized.

**Endpoint**: `POST /auth/setup`

**Authentication**: Not required

**Request Headers**:
```
Content-Type: application/json
```

**Request Body**:
```json
{
  "password": "SecurePassword123!@#"
}
```

**Request Parameters**:
- `password` (string, required): The initial admin password
  - Minimum length: 12 characters
  - Must contain uppercase letter (A-Z)
  - Must contain lowercase letter (a-z)
  - Must contain digit (0-9)
  - Must contain special character (!@#$%^&*)

**Example Request**:
```bash
curl -X POST http://localhost:8080/api/v1/auth/setup \
  -H "Content-Type: application/json" \
  -d '{
    "password": "MySecurePass123!@#"
  }'
```

**Response (Success)**:
```json
{
  "success": true,
  "message": "Admin account initialized successfully",
  "token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
  "expires_at": "2024-03-07T10:30:00Z"
}
```

**Response (Password Policy Violation)**:
```json
{
  "success": false,
  "error": "PasswordPolicyViolation",
  "message": "Password does not meet policy requirements",
  "details": {
    "min_length": "Password must be at least 12 characters",
    "uppercase": "Password must contain at least one uppercase letter",
    "lowercase": "Password must contain at least one lowercase letter",
    "digit": "Password must contain at least one digit",
    "special_char": "Password must contain at least one special character (!@#$%^&*)"
  }
}
```

**Response (Already Initialized)**:
```json
{
  "success": false,
  "error": "AlreadyInitialized",
  "message": "Admin account is already initialized. Use login endpoint instead."
}
```

**Status Codes**:
- `200 OK` - Setup successful
- `400 Bad Request` - Invalid password or already initialized
- `500 Internal Server Error` - Database error

---

### 3. Login

Authenticate with username and password to obtain a session token.

**Endpoint**: `POST /auth/login`

**Authentication**: Not required

**Request Headers**:
```
Content-Type: application/json
```

**Request Body**:
```json
{
  "username": "admin",
  "password": "MySecurePass123!@#"
}
```

**Request Parameters**:
- `username` (string, required): Must be "admin" (fixed username)
- `password` (string, required): The admin password

**Example Request**:
```bash
curl -X POST http://localhost:8080/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{
    "username": "admin",
    "password": "MySecurePass123!@#"
  }'
```

**Response (Success)**:
```json
{
  "success": true,
  "message": "Login successful",
  "token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
  "expires_at": "2024-03-07T10:30:00Z",
  "user": {
    "username": "admin",
    "created_at": "2024-03-06T08:00:00Z"
  }
}
```

**Response (Invalid Credentials)**:
```json
{
  "success": false,
  "error": "AuthenticationFailed",
  "message": "Invalid username or password"
}
```

**Response (Not Initialized)**:
```json
{
  "success": false,
  "error": "NotInitialized",
  "message": "Admin account not yet configured. Please run setup wizard."
}
```

**Status Codes**:
- `200 OK` - Login successful
- `401 Unauthorized` - Invalid credentials
- `400 Bad Request` - Not initialized
- `500 Internal Server Error` - Database error

---

### 4. Change Password

Change the admin password. Requires valid session token.

**Endpoint**: `POST /auth/change-password`

**Authentication**: Required (Bearer token)

**Request Headers**:
```
Content-Type: application/json
Authorization: Bearer <session_token>
```

**Request Body**:
```json
{
  "old_password": "MySecurePass123!@#",
  "new_password": "NewSecurePass456!@#"
}
```

**Request Parameters**:
- `old_password` (string, required): Current admin password
- `new_password` (string, required): New admin password
  - Must meet password policy requirements (same as setup)
  - Cannot be the same as old password

**Example Request**:
```bash
curl -X POST http://localhost:8080/api/v1/auth/change-password \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9..." \
  -d '{
    "old_password": "MySecurePass123!@#",
    "new_password": "NewSecurePass456!@#"
  }'
```

**Response (Success)**:
```json
{
  "success": true,
  "message": "Password changed successfully",
  "token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
  "expires_at": "2024-03-07T10:30:00Z"
}
```

**Response (Invalid Old Password)**:
```json
{
  "success": false,
  "error": "InvalidPassword",
  "message": "Old password is incorrect"
}
```

**Response (Password Policy Violation)**:
```json
{
  "success": false,
  "error": "PasswordPolicyViolation",
  "message": "New password does not meet policy requirements",
  "details": {
    "min_length": "Password must be at least 12 characters",
    "uppercase": "Password must contain at least one uppercase letter",
    "lowercase": "Password must contain at least one lowercase letter",
    "digit": "Password must contain at least one digit",
    "special_char": "Password must contain at least one special character (!@#$%^&*)"
  }
}
```

**Response (Same as Old Password)**:
```json
{
  "success": false,
  "error": "PasswordPolicyViolation",
  "message": "New password cannot be the same as old password"
}
```

**Status Codes**:
- `200 OK` - Password changed successfully
- `400 Bad Request` - Invalid password or policy violation
- `401 Unauthorized` - Invalid or expired token
- `500 Internal Server Error` - Database error

---

### 5. Logout

Invalidate the current session token.

**Endpoint**: `POST /auth/logout`

**Authentication**: Required (Bearer token)

**Request Headers**:
```
Authorization: Bearer <session_token>
```

**Example Request**:
```bash
curl -X POST http://localhost:8080/api/v1/auth/logout \
  -H "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9..."
```

**Response (Success)**:
```json
{
  "success": true,
  "message": "Logout successful"
}
```

**Response (Invalid Token)**:
```json
{
  "success": false,
  "error": "InvalidToken",
  "message": "Invalid or expired token"
}
```

**Status Codes**:
- `200 OK` - Logout successful
- `401 Unauthorized` - Invalid or expired token
- `500 Internal Server Error` - Database error

---

## Error Codes and Handling

### Common Error Codes

| Error Code | HTTP Status | Description | Resolution |
|---|---|---|---|
| `AuthenticationFailed` | 401 | Invalid username or password | Verify credentials and try again |
| `InvalidPassword` | 400 | Old password is incorrect | Verify old password and try again |
| `PasswordPolicyViolation` | 400 | Password doesn't meet requirements | See password policy requirements |
| `InvalidToken` | 401 | Token is invalid or expired | Login again to get a new token |
| `NotInitialized` | 400 | Admin account not configured | Run setup wizard first |
| `AlreadyInitialized` | 400 | Admin account already configured | Use login endpoint |
| `DatabaseError` | 500 | Database operation failed | Check server logs and try again |
| `ServerError` | 500 | Unexpected server error | Check server logs and try again |

### Error Response Format

All error responses follow this format:

```json
{
  "success": false,
  "error": "<error_code>",
  "message": "<human_readable_message>",
  "details": {
    "field": "error_detail"
  }
}
```

---

## Password Policy Requirements

All passwords must meet the following requirements:

| Requirement | Details |
|---|---|
| **Minimum Length** | 12 characters |
| **Uppercase Letters** | At least one uppercase letter (A-Z) |
| **Lowercase Letters** | At least one lowercase letter (a-z) |
| **Digits** | At least one digit (0-9) |
| **Special Characters** | At least one special character (!@#$%^&*) |

### Password Policy Examples

**Valid Passwords**:
- `MySecurePass123!@#`
- `Admin@Password2024`
- `Secure$Pass123abc`
- `P@ssw0rd!Secure`

**Invalid Passwords**:
- `password123` - No uppercase, no special character
- `PASSWORD123!` - No lowercase
- `Pass123!` - Too short (8 characters)
- `MyPassword123` - No special character
- `MyPass!@#` - No digit

---

## Authentication Requirements

### Session Token

Session tokens are JWT-based tokens that expire after a configurable period (default: 24 hours).

**Token Format**:
```
Authorization: Bearer <jwt_token>
```

**Token Claims**:
```json
{
  "sub": "admin",
  "iat": 1709700000,
  "exp": 1709786400,
  "iss": "palpo-admin-server"
}
```

### Token Expiration

- Default expiration: 24 hours
- Expired tokens must be refreshed by logging in again
- Logout invalidates the token immediately

### Protected Endpoints

The following endpoints require a valid session token:
- `POST /auth/change-password`
- `POST /auth/logout`
- All other admin API endpoints (user management, server control, etc.)

---

## Rate Limiting

The authentication endpoints are subject to rate limiting to prevent brute force attacks:

| Endpoint | Rate Limit | Window |
|---|---|---|
| `POST /auth/login` | 5 attempts | 15 minutes |
| `POST /auth/setup` | 1 attempt | N/A (one-time only) |
| `POST /auth/change-password` | 10 attempts | 1 hour |

**Rate Limit Response**:
```json
{
  "success": false,
  "error": "RateLimitExceeded",
  "message": "Too many attempts. Please try again later.",
  "retry_after": 300
}
```

**Status Code**: `429 Too Many Requests`

---

## Audit Logging

All authentication operations are logged to the audit log for security and compliance purposes:

| Operation | Logged Details |
|---|---|
| Setup | Timestamp, success/failure, IP address |
| Login | Timestamp, username, success/failure, IP address |
| Password Change | Timestamp, success/failure, IP address |
| Logout | Timestamp, IP address |

Audit logs can be accessed through the admin dashboard or by querying the `audit_logs` table directly.

---

## Examples

### Complete Login Flow

```bash
# 1. Check initialization status
curl -X GET http://localhost:8080/api/v1/auth/status

# 2. If not initialized, run setup
curl -X POST http://localhost:8080/api/v1/auth/setup \
  -H "Content-Type: application/json" \
  -d '{
    "password": "MySecurePass123!@#"
  }'

# 3. Login with credentials
curl -X POST http://localhost:8080/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{
    "username": "admin",
    "password": "MySecurePass123!@#"
  }'

# 4. Use token for subsequent requests
curl -X POST http://localhost:8080/api/v1/auth/change-password \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <token>" \
  -d '{
    "old_password": "MySecurePass123!@#",
    "new_password": "NewSecurePass456!@#"
  }'

# 5. Logout
curl -X POST http://localhost:8080/api/v1/auth/logout \
  -H "Authorization: Bearer <token>"
```

### Using with JavaScript/TypeScript

```typescript
// Login
const loginResponse = await fetch('http://localhost:8080/api/v1/auth/login', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({
    username: 'admin',
    password: 'MySecurePass123!@#'
  })
});

const { token } = await loginResponse.json();

// Use token for authenticated requests
const changePasswordResponse = await fetch(
  'http://localhost:8080/api/v1/auth/change-password',
  {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'Authorization': `Bearer ${token}`
    },
    body: JSON.stringify({
      old_password: 'MySecurePass123!@#',
      new_password: 'NewSecurePass456!@#'
    })
  }
);
```

---

## Security Considerations

1. **HTTPS Only**: Always use HTTPS in production to protect credentials in transit
2. **Token Storage**: Store tokens securely (e.g., in secure HTTP-only cookies)
3. **Password Storage**: Passwords are hashed using bcrypt/argon2, never stored in plain text
4. **Rate Limiting**: Brute force attacks are mitigated by rate limiting
5. **Audit Logging**: All operations are logged for security auditing
6. **Token Expiration**: Tokens expire after 24 hours for security
7. **CORS**: Configure CORS appropriately for your deployment

---

## Troubleshooting

### "Admin account not yet configured"

**Cause**: The initial password has not been set up.

**Solution**: Call the `/auth/setup` endpoint with a valid password.

### "Invalid username or password"

**Cause**: Incorrect credentials provided.

**Solution**: Verify the username is "admin" and the password is correct.

### "Password does not meet policy requirements"

**Cause**: Password doesn't meet the policy requirements.

**Solution**: Ensure password has at least 12 characters, includes uppercase, lowercase, digit, and special character.

### "Invalid or expired token"

**Cause**: Token is invalid or has expired.

**Solution**: Login again to obtain a new token.

### "Too many attempts"

**Cause**: Rate limit exceeded.

**Solution**: Wait for the specified time before trying again.

---

## API Versioning

This documentation covers API version 1 (`/api/v1`). Future versions may introduce breaking changes.

**Current Version**: v1

**Deprecation Policy**: Deprecated endpoints will be supported for at least 6 months before removal.

