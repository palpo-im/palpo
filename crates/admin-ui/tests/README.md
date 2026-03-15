# E2E Test Scripts

This directory contains end-to-end test scripts for the Palpo Admin UI.

## Available Tests

### 1. Server Control Tests (`e2e_server_control.sh`)

Tests the server control functionality including:
- Server status monitoring
- Server metrics collection
- Server version information
- Server configuration management

**Usage:**

```bash
# Setup environment (start services, keep them running)
bash crates/admin-ui/tests/e2e_server_control.sh --setup

# Run tests (requires services already running)
bash crates/admin-ui/tests/e2e_server_control.sh --test

# Check environment status
bash crates/admin-ui/tests/e2e_server_control.sh --check

# Restart services from scratch
bash crates/admin-ui/tests/e2e_server_control.sh --restart

# Full workflow (setup + test + cleanup)
bash crates/admin-ui/tests/e2e_server_control.sh
```

**Test Cases:**

1. **Get Server Status** - Verify `/api/v1/admin/health/status` endpoint
2. **Get Server Metrics** - Verify `/api/v1/admin/health/metrics` endpoint
3. **Get Server Version** - Verify `/api/v1/admin/health/version` endpoint
4. **Get Server Config** - Verify `/api/v1/admin/server/config` endpoint

### 2. User Management Tests (`e2e_user_management.sh`)

Tests the user management functionality (requires Palpo service running).

**Usage:**

```bash
# Setup environment
bash crates/admin-ui/tests/e2e_user_management.sh --setup

# Run tests
bash crates/admin-ui/tests/e2e_user_management.sh --test

# Clean test data
bash crates/admin-ui/tests/e2e_user_management.sh --clean
```

## Prerequisites

### Required Services

1. **PostgreSQL** - Database server
   ```bash
   # macOS
   brew services start postgresql
   
   # Linux
   sudo systemctl start postgresql
   ```

2. **Admin Server** - Backend API server (started automatically by test scripts)

3. **Admin UI** - Frontend development server (started automatically by test scripts)

### Environment Variables

- `DATABASE_URL` - PostgreSQL connection string (default: `postgresql://palpo:password@localhost/palpo`)
- `ADMIN_PASSWORD` - Admin account password for testing (default: `AdminTest123!`)

## Test Modes

### `--setup`
- Starts all services
- Leaves them running after script exits
- Use this to start the environment once, then run tests multiple times

### `--test`
- Runs tests only
- Requires services already running (via `--setup`)
- Does not start or stop any services

### `--check`
- Checks environment status
- Does not start or stop services
- Useful for verifying all services are ready

### `--clean`
- Cleans test data from database
- Does not start or stop services
- Use between test runs to reset state

### `--restart`
- Kills all services
- Restarts them from scratch
- Leaves services running after script exits

### (default)
- Full workflow: setup + test + cleanup
- Starts services, runs tests, stops services on exit

## Typical Workflow

```bash
# Terminal 1: Start environment once
bash crates/admin-ui/tests/e2e_server_control.sh --setup

# Terminal 2: Run tests multiple times
bash crates/admin-ui/tests/e2e_server_control.sh --test
bash crates/admin-ui/tests/e2e_server_control.sh --test

# Clean test data between runs
bash crates/admin-ui/tests/e2e_server_control.sh --clean

# When done, kill services
pkill -f "palpo-admin-server"
pkill -f "dx serve"
```

## Troubleshooting

### PostgreSQL Connection Failed
```bash
# Check if PostgreSQL is running
pg_isready -h localhost -p 5432

# Start PostgreSQL
brew services start postgresql  # macOS
sudo systemctl start postgresql # Linux
```

### Admin Server Failed to Start
```bash
# Check logs
tail /tmp/admin-server.log

# Verify port 8081 is not in use
lsof -i :8081
```

### Admin UI Failed to Start
```bash
# Check logs
tail /tmp/dioxus.log

# Verify port 8080 is not in use
lsof -i :8080
```

### Tests Failing
```bash
# Check environment status
bash crates/admin-ui/tests/e2e_server_control.sh --check

# Restart everything
bash crates/admin-ui/tests/e2e_server_control.sh --restart

# Run tests again
bash crates/admin-ui/tests/e2e_server_control.sh --test
```

## Test Results

Successful test output:
```
========================================
  Server Control E2E Tests
========================================

[✓] All services are ready!

--- Test 1: Get Server Status ---
[✓] Server status endpoint working
✓ Test 1 PASSED

--- Test 2: Get Server Metrics ---
[✓] Server metrics endpoint working
✓ Test 2 PASSED

--- Test 3: Get Server Version ---
[✓] Server version endpoint working
✓ Test 3 PASSED

--- Test 4: Get Server Config ---
[✓] Server config endpoint working
✓ Test 4 PASSED

========================================
  Test Summary
========================================
  Passed: 4 / 4
========================================
All Server Control tests passed!
```

## Adding New Tests

To add new tests to the script:

1. Add a new test function in the `run_tests()` section
2. Increment `TESTS_PASSED` counter on success
3. Follow the existing test format with clear step descriptions
4. Update the test summary count

Example:
```bash
# Test 5: New Feature
echo "--- Test 5: New Feature ---"
RESULT=$(curl -s "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/new-endpoint")
if echo "$RESULT" | grep -q "expected_field"; then
    log_success "New feature endpoint working"
    TESTS_PASSED=$((TESTS_PASSED + 1))
    echo -e "${GREEN}✓ Test 5 PASSED${NC}"
else
    log_error "New feature endpoint failed"
    echo -e "${RED}✗ Test 5 FAILED${NC}"
fi
echo ""
```

## CI/CD Integration

To integrate these tests into CI/CD:

```bash
# Run full test workflow
bash crates/admin-ui/tests/e2e_server_control.sh

# Exit code 0 = all tests passed
# Exit code 1 = tests failed
```

## Performance Notes

- First run takes longer due to compilation
- Subsequent runs are faster (binaries cached)
- Tests typically complete in 2-3 minutes
- Use `--setup` mode for faster iteration during development
