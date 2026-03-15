# E2E Test Scripts

This directory contains end-to-end test scripts for the Palpo Admin UI.

## Available Tests

### 1. Server Control Tests (`e2e_server_control.sh`)

Tests the server control functionality with both API and browser automation:
- Admin login and authentication (API + UI)
- Server control UI navigation
- Server status monitoring
- Server metrics collection
- Server version information
- Server configuration management

**Features:**
- Automated API testing with curl
- Browser automation with agent-browser (optional)
- Graceful degradation if agent-browser not available
- Comprehensive test reporting

**Usage:**

```bash
# Setup environment (start admin-server and admin-ui)
bash crates/admin-ui/tests/e2e_server_control.sh --setup

# Run automated tests (requires services already running)
bash crates/admin-ui/tests/e2e_server_control.sh --test

# Check environment status
bash crates/admin-ui/tests/e2e_server_control.sh --check

# Restart services from scratch
bash crates/admin-ui/tests/e2e_server_control.sh --restart

# Full workflow (setup + test + cleanup)
bash crates/admin-ui/tests/e2e_server_control.sh
```

**Test Workflow:**

1. **Setup Phase** - Start PostgreSQL, Admin Server, and Admin UI
2. **API Testing Phase** - Verify all endpoints are accessible:
   - `/api/v1/admin/webui-admin/setup` - Setup initial password
   - `/api/v1/admin/webui-admin/login` - Admin login
   - `/api/v1/admin/health/status` - Server status
   - `/api/v1/admin/health/metrics` - Server metrics
   - `/api/v1/admin/health/version` - Server version
   - `/api/v1/admin/server/config` - Server configuration
3. **Browser Testing Phase** (if agent-browser available):
   - Login via UI
   - Navigate to Server Control section
   - Verify server status display
   - Verify metrics display

**Test Results:**

```
========================================
  Test Summary
========================================
  Tests Passed: 10 / 10
========================================
[✓] Core API tests passed!
```

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
- Starts all services (PostgreSQL, Admin Server, Admin UI)
- Leaves them running after script exits
- Use this to start the environment once, then run tests multiple times
- Useful for development and manual testing

### `--test`
- Runs automated tests only
- Requires services already running (via `--setup`)
- Does not start or stop any services
- Includes both API tests (curl) and browser tests (if agent-browser available)
- Gracefully skips browser tests if agent-browser not installed

### `--check`
- Checks environment status
- Does not start or stop services
- Displays which services are ready
- Useful for verifying all services are running

### `--clean`
- Cleans test data from database (audit logs, test admin accounts)
- Removes temporary log files (/tmp/admin-server.log, /tmp/dioxus.log)
- Does not start or stop services
- Use between test runs to reset state

### `--restart`
- Kills all services
- Restarts them from scratch
- Leaves services running after script exits
- Useful for resetting to a clean state

### (default)
- Full workflow: setup + test + cleanup
- Starts services, runs tests, stops services on exit
- Useful for CI/CD pipelines

## Typical Workflow

### Development Workflow (with manual testing)
```bash
# Terminal 1: Start environment once
bash crates/admin-ui/tests/e2e_server_control.sh --setup

# Terminal 2: Run automated tests
bash crates/admin-ui/tests/e2e_server_control.sh --test

# Terminal 2: Open browser for manual testing
# Navigate to http://localhost:8080
# Login with admin credentials (admin / AdminTest123!)
# Test Server Control functionality manually

# When done, kill services
pkill -f "palpo-admin-server"
pkill -f "dx serve"
```

### CI/CD Workflow (fully automated)
```bash
# Run full test workflow (setup + test + cleanup)
bash crates/admin-ui/tests/e2e_server_control.sh

# Exit code 0 = all tests passed
# Exit code 1 = tests failed
```

### Iterative Testing Workflow
```bash
# Terminal 1: Start environment once
bash crates/admin-ui/tests/e2e_server_control.sh --setup

# Terminal 2: Run tests multiple times
bash crates/admin-ui/tests/e2e_server_control.sh --test
bash crates/admin-ui/tests/e2e_server_control.sh --test
bash crates/admin-ui/tests/e2e_server_control.sh --test

# Between test runs, clean test data
bash crates/admin-ui/tests/e2e_server_control.sh --clean

# When done
bash crates/admin-ui/tests/e2e_server_control.sh --restart
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

### Successful API Tests
```
--- API Tests (curl-based) ---

Test 1: Setup Initial Password
[✓] Setup initial password (or already initialized)

Test 2: Login via API
[✓] Login successful (token: a4959c43135a2445940f...)

Test 3: Get Server Status
[✓] Server status endpoint working

Test 4: Get Server Metrics
[✓] Server metrics endpoint working

Test 5: Get Server Version
[✓] Server version endpoint working

Test 6: Get Server Config
[✓] Server config endpoint working
```

### Successful Browser Tests (with agent-browser)
```
--- Browser-based UI Tests ---

Test 7: Login via UI
[✓] UI login successful

Test 8: Navigate to Server Control
[✓] Server Control page loaded

Test 9: Check Server Status Display
[✓] Server status displayed

Test 10: Check Metrics Display
[✓] Metrics displayed
```

### Test Summary
```
========================================
  Test Summary
========================================
  Tests Passed: 10 / 10
========================================
[✓] Core API tests passed!
```

### Browser Tests Not Available
If agent-browser is not installed, the script gracefully skips browser tests:
```
--- Browser-based UI Tests ---

Test 7: Login via UI
[⚠] agent-browser not found, skipping UI tests
[INFO] To enable UI tests, install agent-browser
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

To integrate these tests into CI/CD pipelines:

```bash
# Run full test workflow (automated)
bash crates/admin-ui/tests/e2e_server_control.sh

# Exit code 0 = all tests passed
# Exit code 1 = tests failed
```

### GitHub Actions Example
```yaml
- name: Run E2E Tests
  run: bash crates/admin-ui/tests/e2e_server_control.sh
  timeout-minutes: 10
```

### Notes for CI/CD
- Tests require PostgreSQL to be running
- Tests require ports 8080 and 8081 to be available
- Browser tests (agent-browser) are optional and gracefully skipped if not available
- API tests always run and are required to pass
- Full test suite typically completes in 2-3 minutes

## Performance Notes

- First run takes longer due to compilation
- Subsequent runs are faster (binaries cached)
- Tests typically complete in 2-3 minutes
- Use `--setup` mode for faster iteration during development
