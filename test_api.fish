#!/usr/bin/env fish

# Test Admin Server API
echo "=== Testing Admin Server API ==="

# Step 1: Check status
echo "\n1. Checking admin status..."
set status_response (curl -s http://localhost:8081/api/v1/admin/webui-admin/status)
echo $status_response | jq .

# Step 2: Setup admin if needed
set needs_setup (echo $status_response | jq -r '.needs_setup')
if test "$needs_setup" = "true"
    echo "\n2. Creating admin account..."
    curl -s -X POST http://localhost:8081/api/v1/admin/webui-admin/setup \
      -H "Content-Type: application/json" \
      -d '{"password":"AdminTest123!"}' | jq .
else
    echo "\n2. Admin account already exists"
end

# Step 3: Login to get token
echo "\n3. Logging in..."
set login_response (curl -s -X POST http://localhost:8081/api/v1/admin/webui-admin/login \
  -H "Content-Type: application/json" \
  -d '{"username":"admin","password":"AdminTest123!"}')
echo $login_response | jq .

set TOKEN (echo $login_response | jq -r '.token')

if test -z "$TOKEN" -o "$TOKEN" = "null"
    echo "\n❌ Login failed - no token received"
    exit 1
end

echo "\n✅ Token received: $TOKEN"

# Step 4: Test create user API
echo "\n4. Testing create user API..."
set create_response (curl -s -X POST http://localhost:8081/api/v1/users \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "user_id": "@testuser_api:localhost",
    "displayname": "Test User from API",
    "avatar_url": null,
    "is_admin": false,
    "is_guest": false,
    "user_type": null,
    "appservice_id": null
  }')

echo $create_response | jq .

# Check if user was created successfully
set user_id (echo $create_response | jq -r '.user_id // empty')
if test -n "$user_id"
    echo "\n✅ User created successfully: $user_id"
else
    set error (echo $create_response | jq -r '.error // empty')
    if test -n "$error"
        echo "\n❌ User creation failed: $error"
    else
        echo "\n❌ User creation failed with unknown error"
    end
end

# Step 5: List users
echo "\n5. Listing users..."
curl -s -X GET http://localhost:8081/api/v1/users \
  -H "Authorization: Bearer $TOKEN" | jq .

echo "\n=== Test Complete ==="
