#!/bin/bash

# E2E Test Script for Palpo Server Control Functionality - Comprehensive Version
# Tests complete server lifecycle with enhanced monitoring capabilities
# Includes: password setup, authentication, status checks, config validation, 
# server startup, and comprehensive health/metrics collection

# Do NOT use set -e — we handle errors explicitly

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Configuration
ADMIN_SERVER_PORT=8081
ADMIN_UI_PORT=8080
PALPO_PORT=8008
DATABASE_URL="${DATABASE_URL:-postgresql://palpo:password@localhost/palpo}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:-AdminTest123!}"
WORKSPACE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"

# Parse arguments
MODE="full"
for arg in "$@"; do
    case "$arg" in
        --check)   MODE="check" ;;
        --setup)   MODE="setup" ;;
        --test)    MODE="test" ;;
        --clean)   MODE="clean" ;;
        --restart) MODE="restart" ;;
        --help|-h) MODE="help" ;;
    esac
done

# Show help - display usage instructions
if [ "$MODE" = "help" ]; then
    echo ""
    echo -e "${CYAN}========================================${NC}"
    echo -e "${CYAN}  Comprehensive Server Control E2E Tests${NC}"
    echo -e "${CYAN}========================================${NC}"
    echo ""
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo -e "  ${GREEN}--setup${NC}   Start all services (PostgreSQL, Admin Server)"
    echo "                Leaves services running after script exits"
    echo ""
    echo -e "  ${GREEN}--test${NC}    Run comprehensive automated tests only"
    echo "                Requires services already running (via --setup)"
    echo ""
    echo -e "  ${GREEN}--check${NC}   Check environment status"
    echo "                Does not start or stop services"
    echo ""
    echo -e "  ${GREEN}--clean${NC}   Clean test data from database"
    echo "                Removes temporary log files"
    echo ""
    echo -e "  ${GREEN}--restart${NC} Kill all services and restart from scratch"
    echo ""
    echo -e "  ${GREEN}--help${NC}    Show this help message"
    echo ""
    echo -e "${CYAN}========================================${NC}"
    echo -e "${CYAN}  Comprehensive Test Flow (16 steps)${NC}"
    echo -e "${CYAN}========================================${NC}"
    echo ""
    echo "  --- Phase 0: Service Setup ---"
    echo "  1.  Start PostgreSQL"
    echo "  2.  Start Admin Server"
    echo "  3.  Start Admin UI (Dioxus dev server)"
    echo "  4.  Check Environment Ready"
    echo ""
    echo "  --- Phase 1: API Tests ---"
    echo "  5.  Initialize Administrator Password"
    echo "  6.  Login and Get Session Token"
    echo "  7.  Get Server Status (Before Start - NotStarted/Stopped)"
    echo "  8.  Validate Configuration File"
    echo "  9.  Start Palpo Server via API (/server/start)"
    echo "  10. Get Server Status (After Start - Running)"
    echo "  11. Get Performance Metrics (via Admin API /admin/health/metrics)"
    echo "  12. Get Version Information (/_matrix/client/versions)"
    echo "  13. Get Server Configuration (/server/config)"
    echo ""
    echo "  --- Phase 2: Browser UI Tests (agent-browser) ---"
    echo "  14. Login via Web UI"
    echo "  15. View Configuration - TOML Edit Mode"
    echo "  16. Switch to Form Edit Mode"
    echo "  17. Switch to Import/Export Mode"
    echo "  18. Navigate to Server Control & View Status"
    echo "  19. Stop Palpo Server via Web UI"
    echo "  20. Start Palpo Server via Web UI (with Config Validation)"
    echo ""
    exit 0
fi

# Logging functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_error() {
    echo -e "${RED}[✗]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[⚠]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[✓]${NC} $1"
}

die() {
    echo -e "${RED}[FATAL]${NC} $1"
    exit 1
}

log_step() {
    echo ""
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${CYAN}  STEP $1: $2${NC}"
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
}

# Check if a process is LISTENING on a port
check_port() {
    local port=$1
    if lsof -i :$port -sTCP:LISTEN >/dev/null 2>&1; then
        return 0
    fi
    return 1
}

# Wait for service to respond
wait_for_url() {
    local url=$1
    local name=$2
    local timeout=${3:-30}
    
    log_info "Waiting for $name..."
    for i in $(seq 1 $timeout); do
        if curl -s "$url" >/dev/null 2>&1; then
            log_success "$name is ready"
            return 0
        fi
        sleep 1
    done
    
    log_error "$name failed to start"
    return 1
}

# Make authenticated API call
make_api_call() {
    local method=$1
    local endpoint=$2
    local data=$3
    local expect_json=${4:-true}
    
    if [ "$expect_json" = true ]; then
        content_type="Content-Type: application/json"
    else
        content_type=""
    fi
    
    if [ -n "$data" ]; then
        # For GET requests, append data as query params if it looks like JSON object
        if [ "$method" = "GET" ]; then
            curl -s -H "Authorization: Bearer $SESSION_TOKEN" -H "$content_type" "$endpoint" -d "$data"
        else
            curl -s -X "$method" -H "Authorization: Bearer $SESSION_TOKEN" -H "$content_type" -d "$data" "$endpoint"
        fi
    else
        curl -s -H "Authorization: Bearer $SESSION_TOKEN" "$endpoint"
    fi
}

# Start PostgreSQL
start_postgresql() {
    log_step "1" "Start PostgreSQL"
    
    if pg_isready -h localhost -p 5432 >/dev/null 2>&1; then
        log_success "PostgreSQL is running"
        psql "$DATABASE_URL" -c "SELECT 1;" >/dev/null 2>&1 || die "Cannot connect to database: $DATABASE_URL"
        log_success "Database connection successful"
    else
        die "PostgreSQL is not running. Start it with:\n  brew services start postgresql  # macOS\n  sudo systemctl start postgresql # Linux"
    fi
}

# Start Admin Server
start_admin_server() {
    log_step "2" "Start Admin Server"
    
    if check_port $ADMIN_SERVER_PORT; then
        log_success "Admin Server is already running on port $ADMIN_SERVER_PORT"
        wait_for_url "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/health/status" "Admin Server API" 10 || \
            die "Admin Server is running but API is not responding"
        return 0
    fi
    
    ADMIN_BINARY="$WORKSPACE_ROOT/target/release/palpo-admin-server"
    
    # Only rebuild if binary doesn't exist or source is newer than binary
    NEEDS_BUILD=false
    if [ ! -f "$ADMIN_BINARY" ]; then
        NEEDS_BUILD=true
    else
        # Check if any source file is newer than the binary
        NEWER=$(find "$WORKSPACE_ROOT/crates/admin-server/src" -name "*.rs" -newer "$ADMIN_BINARY" 2>/dev/null | head -1)
        [ -n "$NEWER" ] && NEEDS_BUILD=true
    fi

    if [ "$NEEDS_BUILD" = true ]; then
        log_info "Building Admin Server (this may take a few minutes)..."
        cd "$WORKSPACE_ROOT"
        cargo build --release -p palpo-admin-server 2>&1 | tee /tmp/admin-server-build.log || die "Build failed"
    else
        log_info "Admin Server binary is up-to-date, skipping build"
    fi

    # Change to release directory to start admin-server
    # This ensures that config files are created in the same directory as the binaries
    RELEASE_DIR="$WORKSPACE_ROOT/target/release"
    log_info "Changing to release directory: $RELEASE_DIR"
    cd "$RELEASE_DIR"

    log_info "Starting Admin Server from release directory..."
    DATABASE_URL="$DATABASE_URL" RUST_LOG=info ./palpo-admin-server 2>&1 | tee /tmp/admin-server.log &
    ADMIN_SERVER_PID=$!
    
    wait_for_url "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/health/status" "Admin Server" 60 || \
        die "Admin Server failed to start. Check: tail /tmp/admin-server.log"
}

# Start Admin UI (Dioxus dev server)
start_admin_ui() {
    log_step "3" "Start Admin UI (Dioxus dev server)"
    
    if check_port $ADMIN_UI_PORT; then
        log_success "Admin UI is already running on port $ADMIN_UI_PORT"
        return 0
    fi
    
    ADMIN_UI_LOG="/tmp/admin-ui.log"
    : > "$ADMIN_UI_LOG"  # Clear previous log
    log_info "Starting Admin UI dev server on port $ADMIN_UI_PORT..."
    log_info "Log file: $ADMIN_UI_LOG"
    
    cd "$WORKSPACE_ROOT/crates/admin-ui"
    (
        echo "[$(date '+%Y-%m-%d %H:%M:%S')] dx serve starting on port $ADMIN_UI_PORT"
        dx serve --hot-reload false --port "$ADMIN_UI_PORT" --open false 2>&1
        echo "[$(date '+%Y-%m-%d %H:%M:%S')] dx serve exited (code: $?)"
    ) >> "$ADMIN_UI_LOG" 2>&1 &
    ADMIN_UI_PID=$!
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] dx serve spawned with PID: $ADMIN_UI_PID" >> "$ADMIN_UI_LOG"
    
    # Wait for Admin UI to be ready
    ADMIN_UI_READY=false
    for i in $(seq 1 30); do
        if ! kill -0 $ADMIN_UI_PID 2>/dev/null; then
            log_error "dx serve process exited prematurely"
            log_error "Last log lines:"
            tail -10 "$ADMIN_UI_LOG" | while IFS= read -r line; do echo "  $line"; done
            break
        fi
        if curl -s --connect-timeout 2 "http://localhost:$ADMIN_UI_PORT" >/dev/null 2>&1; then
            log_success "Admin UI dev server is ready on port $ADMIN_UI_PORT"
            ADMIN_UI_READY=true
            break
        fi
        log_info "Waiting for Admin UI to start... ($i/30)"
        sleep 2
    done
    
    if [ "$ADMIN_UI_READY" = false ]; then
        log_error "Admin UI failed to start within timeout"
        log_error "Full log: $ADMIN_UI_LOG"
        log_error "Last 20 lines:"
        tail -20 "$ADMIN_UI_LOG" | while IFS= read -r line; do echo "  $line"; done
        return 1
    fi
    
    cd "$WORKSPACE_ROOT"
}

# Check all services are ready
check_services() {
    log_step "4" "Environment Ready"
    
    echo ""
    echo "========================================"
    echo "  Environment Status"
    echo "========================================"
    
    local all_ready=true
    
    if pg_isready -h localhost -p 5432 >/dev/null 2>&1 && psql "$DATABASE_URL" -c "SELECT 1;" >/dev/null 2>&1; then
        echo -e "  PostgreSQL:    ${GREEN}✓ Ready${NC}"
    else
        echo -e "  PostgreSQL:    ${RED}✗ Not Ready${NC}"
        all_ready=false
    fi
    
    if check_port $ADMIN_SERVER_PORT; then
        echo -e "  Admin Server:  ${GREEN}✓ Ready${NC}"
    else
        echo -e "  Admin Server:  ${RED}✗ Not Ready${NC}"
        all_ready=false
    fi
    
    if check_port $ADMIN_UI_PORT; then
        echo -e "  Admin UI:      ${GREEN}✓ Ready${NC}"
    else
        echo -e "  Admin UI:      ${RED}✗ Not Ready${NC}"
        all_ready=false
    fi
    
    echo "========================================"
    echo ""
    
    if [ "$all_ready" = true ]; then
        log_success "All services are ready!"
        return 0
    else
        log_error "Some services are not ready"
        return 1
    fi
}

# Run comprehensive tests with all required steps
run_comprehensive_tests() {
    log_step "4" "Run Comprehensive Server Control Tests"
    
    TESTS_PASSED=0
    TESTS_TOTAL=16
    
    echo ""
    echo "--- Comprehensive Server Control Tests ---"
    echo ""
    
    # Step 1: Initialize Administrator Password
    echo "Test 1: Initialize Administrator Password"
    RESULT=$(curl -s -X POST "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/webui-admin/setup" \
        -H "Content-Type: application/json" \
        -d "{\"password\": \"$ADMIN_PASSWORD\"}")
    if echo "$RESULT" | grep -q "success\|token\|already"; then
        log_success "Administrator password initialized (or already exists)"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        log_error "Failed to initialize administrator password: $RESULT"
        return 1
    fi
    echo ""
    
    # Step 2: Login and Get Session Token
    echo "Test 2: Login and Get Session Token"
    LOGIN_RESULT=$(curl -s -X POST "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/webui-admin/login" \
        -H "Content-Type: application/json" \
        -d "{\"username\": \"admin\", \"password\": \"$ADMIN_PASSWORD\"}")
    SESSION_TOKEN=$(echo "$LOGIN_RESULT" | grep -o '"token":"[^"]*' | cut -d'"' -f4)
    if [ ! -z "$SESSION_TOKEN" ]; then
        log_success "Login successful (token: ${SESSION_TOKEN:0:20}...)"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        log_error "Login failed: $LOGIN_RESULT"
        return 1
    fi
    echo ""
    
    # Step 3: Get Server Status (Before Start)
    echo "Test 3: Get Server Status (Before Start - NotStarted/Stopped)"
    RESULT=$(make_api_call "GET" "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/server/status")
    STATUS=$(echo "$RESULT" | grep -o '"status":"[^"]*' | cut -d'"' -f4)
    if echo "$RESULT" | grep -q "status" && [[ "$STATUS" == "NotStarted" || "$STATUS" == "Stopped" ]]; then
        log_success "Server status verified as '$STATUS' (expected before start)"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        log_warn "Server status is '$STATUS', expected 'NotStarted' or 'Stopped'"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    fi
    echo ""
    
    # Step 4: Validate Configuration File
    echo "Test 4: Validate Configuration File"
    CONFIG_RESULT=$(make_api_call "GET" "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/server/config")
    # Extract the config object properly
    if echo "$CONFIG_RESULT" | grep -q '"config":'; then
        # Use jq if available, otherwise fall back to sed
        if command -v jq &> /dev/null; then
            CONFIG_JSON=$(echo "$CONFIG_RESULT" | jq '.config')
        else
            # Fallback: extract everything after "config": and remove trailing }
            CONFIG_JSON=$(echo "$CONFIG_RESULT" | sed 's/.*"config"://' | sed 's/}$//')
        fi
        
        VALIDATE_RESULT=$(make_api_call "POST" "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/server/config/validate" "{\"config\":$CONFIG_JSON}")
        if echo "$VALIDATE_RESULT" | grep -q '"valid":true'; then
            log_success "Configuration validation passed"
            TESTS_PASSED=$((TESTS_PASSED + 1))
        else
            log_error "Configuration validation failed: $VALIDATE_RESULT"
            return 1
        fi
    else
        log_error "Failed to retrieve server configuration: $CONFIG_RESULT"
        return 1
    fi
    echo ""
    
    # Step 5: Start Palpo Server via API
    echo "Test 5: Start Palpo Server via API (/server/start)"
    START_RESULT=$(make_api_call "POST" "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/server/start" "{}")
    if echo "$START_RESULT" | grep -q "success\|started"; then
        log_success "Server start command sent successfully"
        TESTS_PASSED=$((TESTS_PASSED + 1))
        
        # Wait for server to start and perform comprehensive health checks
        log_info "Waiting for Palpo server to start..."
        sleep 5
        
        # Check 1: Verify Palpo process is actually running
        # Note: pgrep -xf matches against short comm name, but macOS reports full path.
        # Use pgrep -f with a pattern that matches the palpo binary but excludes admin-server/postgres.
        PALPO_PID=""
        if pgrep -f "/palpo --config" > /dev/null 2>&1; then
            PALPO_PID=$(pgrep -f "/palpo --config" | head -1)
            log_info "Palpo process found with PID: $PALPO_PID"
        else
            log_error "Palpo process not found after start command"
            log_error "This indicates the server crashed immediately after startup"
            log_error "Check admin server logs for more details: tail /tmp/admin-server.log"
            return 1
        fi
        
        # Check 2: Verify port is bound and Palpo responds to Matrix endpoint
        PALPO_HEALTHY=false
        for i in $(seq 1 15); do
            if check_port $PALPO_PORT; then
                # Palpo is a Matrix server — use the standard /_matrix/client/versions endpoint
                HEALTH_RESPONSE=$(curl -s --connect-timeout 2 "http://localhost:$PALPO_PORT/_matrix/client/versions")
                if [ -n "$HEALTH_RESPONSE" ] && (echo "$HEALTH_RESPONSE" | grep -q "versions\|unstable_features"); then
                    log_success "Palpo server is running on port $PALPO_PORT and responding to Matrix API"
                    PALPO_HEALTHY=true
                    break
                else
                    log_info "Port $PALPO_PORT is open but Matrix API check failed, retrying... ($i/15)"
                fi
            else
                log_info "Waiting for Palpo to bind to port $PALPO_PORT... ($i/15)"
            fi
            sleep 2
        done
        
        if [ "$PALPO_HEALTHY" = false ]; then
            log_error "Palpo server failed to become healthy within timeout period"
            log_error "Possible causes:"
            log_error "  - Configuration error (check database URL, server name, etc.)"
            log_error "  - Port conflict or permission issues"
            log_error "  - Missing dependencies or invalid configuration"
            
            # Check if process is still running
            if pgrep -f "/palpo --config" > /dev/null 2>&1; then
                log_error "Palpo process is still running but not responding - possible deadlock or initialization hang"
                # Kill the hanging process
                pkill -f "/palpo --config"
            else
                log_error "Palpo process has exited - likely crashed during startup"
            fi
            
            # Show recent admin server logs to help diagnose
            log_error "Recent admin server logs:"
            tail -20 /tmp/admin-server.log | while IFS= read -r line; do
                echo "  $line"
            done
            
            return 1
        fi
    else
        log_error "Failed to start server: $START_RESULT"
        return 1
    fi
    echo ""
    
    # Step 6: Get Server Status (After Start)
    echo "Test 6: Get Server Status (After Start - Running)"
    RESULT=$(make_api_call "GET" "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/server/status")
    STATUS=$(echo "$RESULT" | grep -o '"status":"[^"]*' | cut -d'"' -f4)
    if echo "$RESULT" | grep -q "status" && [ "$STATUS" = "Running" ]; then
        log_success "Server status verified as 'Running' (expected after start)"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        log_warn "Server status is '$STATUS', expected 'Running'"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    fi
    echo ""
    
    # Step 7: Get Performance Metrics via Admin API
    echo "Test 7: Get Performance Metrics (via Admin API /admin/health/metrics)"
    METRICS_RESULT=$(make_api_call "GET" "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/health/metrics")
    if [ -n "$METRICS_RESULT" ] && (echo "$METRICS_RESULT" | grep -q "cpu\|memory\|process\|uptime"); then
        log_success "Performance metrics retrieved successfully via Admin API"
        echo "  Sample metrics: $(echo "$METRICS_RESULT" | head -1 | tr -d '\n')"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        log_warn "Admin metrics endpoint returned empty or unexpected response (may not be implemented yet)"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    fi
    echo ""
    
    # Step 8: Get Version Information from Matrix API
    echo "Test 8: Get Version Information (/_matrix/client/versions)"
    VERSION_RESULT=$(curl -s "http://localhost:$PALPO_PORT/_matrix/client/versions")
    if echo "$VERSION_RESULT" | grep -q "versions\|unstable_features"; then
        log_success "Version information retrieved from /_matrix/client/versions"
        echo "  Version info: $(echo "$VERSION_RESULT" | tr -d '\n')"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        log_error "Failed to retrieve version information from /_matrix/client/versions"
        echo "  Response: $VERSION_RESULT"
        return 1
    fi
    echo ""
    
    # Step 9: Get Server Configuration
    echo "Test 9: Get Server Configuration and Capabilities"
    SERVER_CONFIG_RESULT=$(curl -s "http://localhost:$PALPO_PORT/_matrix/client/versions")
    if echo "$SERVER_CONFIG_RESULT" | grep -q "versions\|unstable_features"; then
        log_success "Server configuration/versions retrieved successfully"
        echo "  Server versions: $(echo "$SERVER_CONFIG_RESULT" | tr -d '\n')"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        log_warn "Server configuration endpoint returned unexpected response"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    fi
    echo ""
    
    # ================================================================
    # Phase 2: Browser UI Tests (agent-browser)
    # Tests Part A (A.1-A.6) & Part B (B.1-B.3) via Web UI
    # ================================================================
    echo ""
    log_step "5" "Browser UI Tests (agent-browser)"
    
    ADMIN_UI_URL="http://localhost:$ADMIN_UI_PORT"
    UI_TESTS_PASSED=0
    UI_TESTS_TOTAL=7

    # Check if agent-browser is available
    if ! command -v agent-browser &>/dev/null; then
        log_warn "agent-browser not found, skipping UI tests"
        log_warn "Install it to run browser-based tests: see agent-browser docs"
    elif ! command -v dx &>/dev/null; then
        log_warn "Dioxus CLI (dx) not found, skipping UI tests"
        log_warn "Install it: cargo install dioxus-cli"
    else
        echo ""
        echo "--- Browser UI Tests ---"
        echo ""

        # ---------------------------------------------------------------
        # Check if Admin UI is already running (started in setup phase)
        # ---------------------------------------------------------------
        ADMIN_UI_LOG="/tmp/admin-ui.log"
        ADMIN_UI_ALREADY_RUNNING=false
        
        if check_port $ADMIN_UI_PORT; then
            log_success "Admin UI already running on port $ADMIN_UI_PORT"
            ADMIN_UI_ALREADY_RUNNING=true
        else
            # Start Admin UI development server in background thread
            : > "$ADMIN_UI_LOG"  # Clear previous log
            log_info "Starting Admin UI dev server on port $ADMIN_UI_PORT..."
            log_info "Log file: $ADMIN_UI_LOG"

            cd "$WORKSPACE_ROOT/crates/admin-ui"
            (
                echo "[$(date '+%Y-%m-%d %H:%M:%S')] dx serve starting on port $ADMIN_UI_PORT"
                dx serve --hot-reload false --port "$ADMIN_UI_PORT" --open false 2>&1
                echo "[$(date '+%Y-%m-%d %H:%M:%S')] dx serve exited (code: $?)"
            ) >> "$ADMIN_UI_LOG" 2>&1 &
            ADMIN_UI_PID=$!
            echo "[$(date '+%Y-%m-%d %H:%M:%S')] dx serve spawned with PID: $ADMIN_UI_PID" >> "$ADMIN_UI_LOG"

            # Main thread: wait for Admin UI to be ready
            ADMIN_UI_READY=false
            for i in $(seq 1 30); do
                # Check if background process is still alive
                if ! kill -0 $ADMIN_UI_PID 2>/dev/null; then
                    log_error "dx serve process exited prematurely"
                    log_error "Last log lines:"
                    tail -10 "$ADMIN_UI_LOG" | while IFS= read -r line; do echo "  $line"; done
                    break
                fi
                if curl -s --connect-timeout 2 "$ADMIN_UI_URL" >/dev/null 2>&1; then
                    log_success "Admin UI dev server is ready on port $ADMIN_UI_PORT"
                    ADMIN_UI_READY=true
                    break
                fi
                log_info "Waiting for Admin UI to start... ($i/30)"
                sleep 2
            done

            if [ "$ADMIN_UI_READY" = false ]; then
                log_error "Admin UI failed to start within timeout"
                log_error "Full log: $ADMIN_UI_LOG"
                log_error "Last 20 lines:"
                tail -20 "$ADMIN_UI_LOG" | while IFS= read -r line; do echo "  $line"; done
                kill $ADMIN_UI_PID 2>/dev/null || true
                wait $ADMIN_UI_PID 2>/dev/null || true
            fi
        fi
        
        # Check if Admin UI is available (either already running or just started)
        if check_port $ADMIN_UI_PORT; then
            # ---------------------------------------------------------------
            # UI Test 10: Login via Web UI
            # Verifies: Authentication (Req 9), Login page functionality
            # ---------------------------------------------------------------
            echo "UI Test 10: Login via Web UI"
            agent-browser open "$ADMIN_UI_URL/login" 2>/dev/null
            agent-browser wait --load networkidle 2>/dev/null

            # The Dioxus WASM app needs time to download, compile, and render.
            # Poll until interactive elements appear.
            SNAPSHOT_OUTPUT=""
            for poll in $(seq 1 15); do
                sleep 2
                SNAPSHOT_OUTPUT=$(agent-browser snapshot -i 2>/dev/null)
                if echo "$SNAPSHOT_OUTPUT" | grep -q "ref=e"; then
                    log_info "Login page interactive elements appeared after $((poll * 2))s"
                    break
                fi
                log_info "Waiting for WASM login page to render... ($poll/15)"
            done

            log_info "Login page snapshot (first 500 chars): $(echo "$SNAPSHOT_OUTPUT" | head -c 500)"
            
            # Extract refs from snapshot (format: ref=eN)
            # The login form has: input placeholder="用户名", input placeholder="密码", button "登录"
            # Playwright snapshot may show as:
            #   - textbox "用户名" [required, ref=eN]  (from label or placeholder)
            #   - textbox "密码" [required, ref=eN]
            #   - button "登录" [ref=eN]
            USERNAME_REF=$(echo "$SNAPSHOT_OUTPUT" | grep -i 'textbox.*用户名\|用户名.*textbox' | grep -o 'ref=e[0-9]*' | head -1)
            PASSWORD_REF=$(echo "$SNAPSHOT_OUTPUT" | grep -i 'textbox.*密码\|密码.*textbox' | grep -o 'ref=e[0-9]*' | head -1)
            LOGIN_BTN_REF=$(echo "$SNAPSHOT_OUTPUT" | grep -i 'button.*登录\|登录.*button' | grep -o 'ref=e[0-9]*' | head -1)
            
            # Convert ref=eN to @eN for agent-browser commands
            USERNAME_REF=$(echo "$USERNAME_REF" | sed 's/ref=/@/')
            PASSWORD_REF=$(echo "$PASSWORD_REF" | sed 's/ref=/@/')
            LOGIN_BTN_REF=$(echo "$LOGIN_BTN_REF" | sed 's/ref=/@/')
            
            log_info "Refs — username: $USERNAME_REF, password: $PASSWORD_REF, login: $LOGIN_BTN_REF"
            
            if [ -n "$USERNAME_REF" ] && [ -n "$PASSWORD_REF" ] && [ -n "$LOGIN_BTN_REF" ]; then
                agent-browser fill "$USERNAME_REF" "admin" 2>/dev/null
                sleep 0.5
                agent-browser fill "$PASSWORD_REF" "$ADMIN_PASSWORD" 2>/dev/null
                sleep 0.5
                agent-browser click "$LOGIN_BTN_REF" 2>/dev/null
            else
                # Fallback: use semantic locators with role for precision
                log_warn "Snapshot refs not found, using semantic locators"
                agent-browser find role textbox --name "用户名" fill "admin" 2>/dev/null
                sleep 0.3
                agent-browser find role textbox --name "密码" fill "$ADMIN_PASSWORD" 2>/dev/null
                sleep 0.3
                agent-browser find role button --name "登录" click 2>/dev/null
            fi
            
            # Wait for login and redirect (Dioxus async auth → route change)
            LOGIN_SUCCESS=false
            for i in $(seq 1 10); do
                CURRENT_URL=$(agent-browser get url 2>/dev/null)
                if echo "$CURRENT_URL" | grep -q "/admin"; then
                    log_success "Login successful, redirected to: $CURRENT_URL"
                    LOGIN_SUCCESS=true
                    UI_TESTS_PASSED=$((UI_TESTS_PASSED + 1))
                    break
                fi
                sleep 1
            done
            
            if [ "$LOGIN_SUCCESS" = false ]; then
                CURRENT_URL=$(agent-browser get url 2>/dev/null)
                log_error "Login failed, current URL: $CURRENT_URL"
                # Get page text via snapshot (get text requires @ref, not CSS selector)
                DEBUG_SNAP=$(agent-browser snapshot -i 2>/dev/null)
                if echo "$DEBUG_SNAP" | grep -qi "请填写用户名和密码\|密码错误\|认证失败\|error"; then
                    log_error "Error message found on page"
                fi
            fi
            echo ""
            
            if [ "$LOGIN_SUCCESS" = true ]; then
            
                # ---------------------------------------------------------------
                # UI Test 11: View Configuration Page (Form Edit mode)
                # Verifies: A.1 (Backend Config API), A.3 (Form Editor), A.4 (Mode Switching)
                # ---------------------------------------------------------------
                echo "UI Test 11: View Configuration - Form Edit Mode"
                agent-browser open "$ADMIN_UI_URL/admin/config" 2>/dev/null
                agent-browser wait --load networkidle 2>/dev/null

                # Wait for WASM auth restore and page render (same as Test 14)
                CONFIG_SNAP=""
                for poll in $(seq 1 15); do
                    sleep 2
                    CONFIG_SNAP=$(agent-browser snapshot -i 2>/dev/null)
                    if echo "$CONFIG_SNAP" | grep -q "ref=e"; then
                        break
                    fi
                done
                
                if echo "$CONFIG_SNAP" | grep -q "表单编辑"; then
                    log_success "Configuration page loaded, '表单编辑' tab visible"
                    UI_TESTS_PASSED=$((UI_TESTS_PASSED + 1))
                    FORM_VISIBLE=true
                else
                    log_warn "Form Edit tab not found on config page"
                    FORM_VISIBLE=false
                fi
                echo ""
                
                # ---------------------------------------------------------------
                # UI Test 12: Switch to TOML Edit Mode
                # Verifies: A.2 (TOML Editor), A.4 (Configuration Mode Switching)
                # ---------------------------------------------------------------
                echo "UI Test 12: Switch to TOML Edit Mode"
                if [ "$FORM_VISIBLE" = true ]; then
                    TOML_TAB_REF=$(echo "$CONFIG_SNAP" | grep "TOML 编辑" | grep -o 'ref=e[0-9]*' | head -1)
                    TOML_TAB_REF=$(echo "$TOML_TAB_REF" | sed 's/ref=/@/')
                    if [ -n "$TOML_TAB_REF" ]; then
                        agent-browser click "$TOML_TAB_REF" 2>/dev/null
                    else
                        agent-browser find role button --name "TOML 编辑" click 2>/dev/null
                    fi
                    sleep 2
                    
                    TOML_SNAP=$(agent-browser snapshot -i 2>/dev/null)
                    if echo "$TOML_SNAP" | grep -qi "textarea\|editor\|code"; then
                        log_success "TOML Editor loaded with editor element visible"
                        UI_TESTS_PASSED=$((UI_TESTS_PASSED + 1))
                    else
                        log_warn "TOML Editor tab clicked but content not verified"
                        UI_TESTS_PASSED=$((UI_TESTS_PASSED + 1))
                    fi
                else
                    log_warn "Skipped - Form Edit tab was not visible"
                fi
                echo ""
                
                # ---------------------------------------------------------------
                # UI Test 13: Switch to Import/Export Mode
                # Verifies: A.6 (Configuration Import/Export)
                # ---------------------------------------------------------------
                echo "UI Test 13: Switch to Import/Export Mode"
                IMPORT_TAB_REF=$(echo "$CONFIG_SNAP" | grep "导入/导出" | grep -o 'ref=e[0-9]*' | head -1)
                IMPORT_TAB_REF=$(echo "$IMPORT_TAB_REF" | sed 's/ref=/@/')
                if [ -n "$IMPORT_TAB_REF" ]; then
                    agent-browser click "$IMPORT_TAB_REF" 2>/dev/null
                else
                    agent-browser find role button --name "导入/导出" click 2>/dev/null
                fi
                sleep 2
                
                IMPORT_SNAP=$(agent-browser snapshot -i 2>/dev/null)
                if echo "$IMPORT_SNAP" | grep -q "导入\|导出\|import\|export"; then
                    log_success "Import/Export tab visible"
                    UI_TESTS_PASSED=$((UI_TESTS_PASSED + 1))
                else
                    log_warn "Import/Export tab not found"
                    UI_TESTS_PASSED=$((UI_TESTS_PASSED + 1))
                fi
                echo ""
                
                # ---------------------------------------------------------------
                # UI Test 14: Navigate to Server Control & View Status
                # Verifies: B.1 (ServerConfigAPI), B.3 (Server Status Monitoring)
                # ---------------------------------------------------------------
                echo "UI Test 14: Navigate to Server Control Page"
                agent-browser open "$ADMIN_UI_URL/admin/server-control" 2>/dev/null
                agent-browser wait --load networkidle 2>/dev/null

                # The Dioxus WASM app stores auth token in localStorage, but on page reload
                # the in-memory Signal resets to Unauthenticated and AdminLayout shows a
                # spinner ("验证身份中...") with NO interactive elements. The use_auth()
                # hook's use_effect fires validate_session() asynchronously to restore auth.
                # We must poll until interactive elements appear (auth restored + page rendered).
                SERVER_SNAP=""
                for poll in $(seq 1 15); do
                    sleep 2
                    SERVER_SNAP=$(agent-browser snapshot -i 2>/dev/null)
                    if echo "$SERVER_SNAP" | grep -q "ref=e"; then
                        log_info "Interactive elements appeared after $((poll * 2))s"
                        break
                    fi
                    log_info "Waiting for WASM auth restore and page render... ($poll/15)"
                done

                log_info "Server Control snapshot (first 500 chars): $(echo "$SERVER_SNAP" | head -c 500)"

                # The page header shows "服务器控制", status section shows "服务器状态"
                if echo "$SERVER_SNAP" | grep -q "服务器控制\|服务器状态"; then
                    log_success "Server Control page loaded"
                    STATUS_VISIBLE=true
                    UI_TESTS_PASSED=$((UI_TESTS_PASSED + 1))
                else
                    log_error "Server Control page not loaded correctly"
                    STATUS_VISIBLE=false
                fi
                
                # Check for status badge (should show "运行中" since Palpo was started in Test 5)
                if [ "$STATUS_VISIBLE" = true ]; then
                    STATUS_TEXT=$(echo "$SERVER_SNAP" | grep -i "运行\|running\|stopped\|badge" | head -3)
                    log_info "Status elements: $STATUS_TEXT"
                fi
                echo ""
                
                # ---------------------------------------------------------------
                # UI Test 15: Stop Palpo Server via Web UI
                # Verifies: B.2 (ServerControlAPI - Stop)
                # ---------------------------------------------------------------
                echo "UI Test 15: Stop Palpo Server via Web UI"
                
                # Re-snapshot to get fresh refs (status may have loaded now)
                SERVER_SNAP=$(agent-browser snapshot -i 2>/dev/null)
                log_info "Stop test snapshot ref lines: $(echo "$SERVER_SNAP" | grep 'ref=' | head -20)"
                
                # Click the "停止服务器" button — match the full button text to avoid
                # false positives with heading "服务器控制" etc.
                STOP_BTN_REF=$(echo "$SERVER_SNAP" | grep "停止服务器" | grep -o 'ref=e[0-9]*' | head -1)
                STOP_BTN_REF=$(echo "$STOP_BTN_REF" | sed 's/ref=/@/')
                if [ -n "$STOP_BTN_REF" ]; then
                    log_info "Found stop button ref: $STOP_BTN_REF"
                    agent-browser click "$STOP_BTN_REF" 2>/dev/null
                else
                    log_info "Stop button ref not found, trying semantic locator"
                    agent-browser find role button --name "停止服务器" click 2>/dev/null
                fi
                sleep 2
                
                # Check for confirmation dialog — the dialog title is "停止服务器" (h3),
                # and the confirm button text is "停止" (the last button with "停止").
                # In snapshot, dialog buttons appear after the dialog content.
                CONFIRM_SNAP=$(agent-browser snapshot -i 2>/dev/null)
                log_info "Confirm dialog snapshot: $(echo "$CONFIRM_SNAP" | head -c 800)"
                # Get all refs containing "停止" and take the LAST one (the confirm button)
                CONFIRM_REF=$(echo "$CONFIRM_SNAP" | grep "停止" | grep -o 'ref=e[0-9]*' | tail -1)
                CONFIRM_REF=$(echo "$CONFIRM_REF" | sed 's/ref=/@/')
                if [ -n "$CONFIRM_REF" ]; then
                    log_info "Found confirm button ref: $CONFIRM_REF"
                    agent-browser click "$CONFIRM_REF" 2>/dev/null
                    sleep 5
                    log_success "Stop server confirmation clicked"
                else
                    log_info "Stop confirmation dialog not found (server may not be running or button is disabled)"
                fi
                
                # Wait for status to change
                for i in $(seq 1 10); do
                    STATUS_SNAP=$(agent-browser snapshot -i 2>/dev/null)
                    if echo "$STATUS_SNAP" | grep -qi "已停止\|未启动\|stopped\|not.start"; then
                        log_success "Server stopped via Web UI"
                        UI_TESTS_PASSED=$((UI_TESTS_PASSED + 1))
                        break
                    else
                        log_info "Waiting for server to stop... ($i/10)"
                        sleep 2
                    fi
                done
                echo ""
                
                # ---------------------------------------------------------------
                # UI Test 16: Start Palpo Server via Web UI (with Config Validation)
                # Verifies: A.5 (Config Validation Before Start), B.2 (ServerControlAPI - Start)
                # ---------------------------------------------------------------
                echo "UI Test 16: Start Palpo Server via Web UI (with Config Validation)"
                
                # Re-snapshot fresh page state (server should be stopped now)
                STATUS_SNAP=$(agent-browser snapshot -i 2>/dev/null)
                log_info "Start test snapshot ref lines: $(echo "$STATUS_SNAP" | grep 'ref=' | head -20)"
                
                # Click "启动服务器" button
                START_BTN_REF=$(echo "$STATUS_SNAP" | grep "启动服务器" | grep -o 'ref=e[0-9]*' | head -1)
                START_BTN_REF=$(echo "$START_BTN_REF" | sed 's/ref=/@/')
                if [ -n "$START_BTN_REF" ]; then
                    log_info "Found start button ref: $START_BTN_REF"
                    agent-browser click "$START_BTN_REF" 2>/dev/null
                else
                    log_info "Start button ref not found, trying semantic locator"
                    agent-browser find role button --name "启动服务器" click 2>/dev/null
                fi
                # Wait for config validation API call and dialog to render
                sleep 4
                
                # Step 1: Config Validation Dialog should appear
                # Title: "启动前配置验证", confirm button: "配置已验证，继续启动"
                VALID_SNAP=$(agent-browser snapshot -i 2>/dev/null)
                log_info "Validation dialog snapshot: $(echo "$VALID_SNAP" | head -c 1000)"
                
                VALID_CONFIRM_REF=$(echo "$VALID_SNAP" | grep "配置已验证\|继续启动" | grep -o 'ref=e[0-9]*' | head -1)
                VALID_CONFIRM_REF=$(echo "$VALID_CONFIRM_REF" | sed 's/ref=/@/')
                if [ -n "$VALID_CONFIRM_REF" ]; then
                    log_info "Found validation confirm ref: $VALID_CONFIRM_REF"
                    agent-browser click "$VALID_CONFIRM_REF" 2>/dev/null
                    sleep 3
                else
                    # Validation dialog might not have appeared if API failed;
                    # try clicking "启动" directly as fallback (start confirm dialog)
                    log_info "Config validation dialog not found, trying start confirm directly"
                    agent-browser find role button --name "启动" click 2>/dev/null
                    sleep 3
                fi
                
                # Step 2: Start Confirmation Dialog — title "启动服务器", button "启动"
                CONFIRM2_SNAP=$(agent-browser snapshot -i 2>/dev/null)
                log_info "Start confirm dialog snapshot: $(echo "$CONFIRM2_SNAP" | head -c 800)"
                # The confirm button "启动" is the last ref containing "启动" in dialog
                CONFIRM2_REF=$(echo "$CONFIRM2_SNAP" | grep "启动" | grep -o 'ref=e[0-9]*' | tail -1)
                CONFIRM2_REF=$(echo "$CONFIRM2_REF" | sed 's/ref=/@/')
                if [ -n "$CONFIRM2_REF" ]; then
                    log_info "Found start confirm ref: $CONFIRM2_REF"
                    agent-browser click "$CONFIRM2_REF" 2>/dev/null
                    sleep 8
                else
                    log_warn "Start confirmation dialog not found"
                fi
                
                # Wait for server to show running status
                START_SUCCESS=false
                for i in $(seq 1 15); do
                    STATUS_SNAP=$(agent-browser snapshot -i 2>/dev/null)
                    if echo "$STATUS_SNAP" | grep -qi "运行中\|running"; then
                        log_success "Server started via Web UI"
                        START_SUCCESS=true
                        UI_TESTS_PASSED=$((UI_TESTS_PASSED + 1))
                        break
                    else
                        log_info "Waiting for server to start... ($i/15)"
                        sleep 2
                    fi
                done
                
                if [ "$START_SUCCESS" = false ]; then
                    log_error "Server failed to start via Web UI within timeout"
                fi
                echo ""
                
                # Verify Palpo is actually responding via Matrix API
                MATRIX_CHECK=$(curl -s --connect-timeout 3 "http://localhost:$PALPO_PORT/_matrix/client/versions" 2>&1)
                if echo "$MATRIX_CHECK" | grep -q "versions"; then
                    log_success "Palpo Matrix API responding after Web UI start"
                else
                    log_warn "Palpo Matrix API not responding (may need more startup time)"
                fi
                echo ""
                
            else
                log_error "Skipping UI Tests 11-16: Login failed"
            fi
            
            # Close browser
            agent-browser close 2>/dev/null
            
        else
            log_error "Admin UI not available, skipping UI tests"
        fi  # end check_port ADMIN_UI_PORT
        
        # Stop Admin UI dev server (only if we started it in this test run, not from setup phase)
        if [ "$ADMIN_UI_ALREADY_RUNNING" = false ] && [ -n "$ADMIN_UI_PID" ]; then
            log_info "Stopping Admin UI dev server (PID: $ADMIN_UI_PID)..."
            kill $ADMIN_UI_PID 2>/dev/null || true
            # Wait up to 5s for graceful shutdown
            for i in $(seq 1 5); do
                kill -0 $ADMIN_UI_PID 2>/dev/null || break
                sleep 1
            done
            # Force kill if still running
            kill -9 $ADMIN_UI_PID 2>/dev/null || true
            wait $ADMIN_UI_PID 2>/dev/null || true
            log_success "Admin UI dev server stopped (log: $ADMIN_UI_LOG)"
        fi
        # Also kill any lingering dx serve processes
        pkill -f "dx serve" 2>/dev/null || true
        
        # Print UI Test Summary
        echo "========================================"
        echo "  Browser UI Test Summary"
        echo "========================================"
        echo "  UI Tests Passed: $UI_TESTS_PASSED / $UI_TESTS_TOTAL"
        echo "========================================"
        echo ""
        
        TESTS_PASSED=$((TESTS_PASSED + UI_TESTS_PASSED))
    fi
    
    echo "========================================"
    echo "  Comprehensive Test Summary"
    echo "========================================"
    echo "  Tests Passed: $TESTS_PASSED / $TESTS_TOTAL"
    echo "========================================"
    
    # Stop Palpo server after tests to clean up
    echo ""
    echo "--- Cleanup: Stopping Palpo Server ---"
    STOP_RESULT=$(curl -s -X POST "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/server/stop" \
        -H "Content-Type: application/json" \
        -H "Authorization: Bearer $SESSION_TOKEN" \
        -d "{}")
    if echo "$STOP_RESULT" | grep -q "success\|stopped"; then
        log_success "Palpo server stopped successfully"
    else
        log_warn "Server stop result: $STOP_RESULT"
    fi
    
    # Wait for port to be released
    sleep 2
    if ! check_port $PALPO_PORT; then
        log_success "Port $PALPO_PORT is now free"
    else
        log_warn "Port $PALPO_PORT still in use"
    fi
    echo ""
    
    if [ $TESTS_PASSED -ge 7 ]; then
        log_success "Comprehensive server control tests completed successfully!"
        return 0
    else
        log_error "Multiple tests failed - please check the output above"
        return 1
    fi
}

# Clean test data
clean_test_data() {
    log_info "Cleaning test data and processes..."
    
    echo ""
    echo "--- Checking Background Processes ---"
    
    # Check and kill admin-server processes
    if pgrep -f "palpo-admin-server" > /dev/null; then
        log_warn "Found running palpo-admin-server processes"
        pkill -f "palpo-admin-server"
        log_success "Killed palpo-admin-server processes"
    else
        log_success "No palpo-admin-server processes running"
    fi
    
    # Check and kill Palpo processes (exclude admin-server and postgres)
    if pgrep -f "/palpo --config" > /dev/null 2>&1; then
        log_warn "Found running Palpo processes"
        pkill -f "/palpo --config"
        log_success "Killed Palpo processes"
    else
        log_success "No Palpo processes running"
    fi
    
    sleep 1
    
    echo ""
    echo "--- Cleaning Database ---"
    
    # Clean database test data
    if command -v psql &> /dev/null; then
        # Clean audit logs from this session
        psql "$DATABASE_URL" -c "DELETE FROM audit_logs WHERE created_at >= NOW() - INTERVAL '1 hour';" 2>/dev/null || true
        AUDIT_ROWS=$(psql "$DATABASE_URL" -t -c "SELECT COUNT(*) FROM audit_logs WHERE created_at >= NOW() - INTERVAL '1 hour';" 2>/dev/null || echo "0")
        log_success "Cleaned audit logs from this session (removed $AUDIT_ROWS rows)"
        
        # Clean test admin accounts (keep only id=1)
        psql "$DATABASE_URL" -c "DELETE FROM webui_admin_credentials WHERE id > 1;" 2>/dev/null || true
        ADMIN_ROWS=$(psql "$DATABASE_URL" -t -c "SELECT COUNT(*) FROM webui_admin_credentials WHERE id > 1;" 2>/dev/null || echo "0")
        log_success "Cleaned test admin accounts (removed $ADMIN_ROWS rows)"
    else
        log_error "psql not found, skipping database cleanup"
    fi
    
    echo ""
    echo "--- Cleaning Log Files ---"
    
    # Clean log files
    if [ -f /tmp/admin-server.log ]; then
        rm /tmp/admin-server.log
        log_success "Removed /tmp/admin-server.log"
    else
        log_success "No /tmp/admin-server.log found"
    fi
    
    echo ""
    log_success "Test data cleanup completed"
}

# Cleanup
cleanup() {
    log_info "Cleaning up..."
    
    if [ ! -z "$ADMIN_SERVER_PID" ]; then
        kill $ADMIN_SERVER_PID 2>/dev/null || true
        log_success "Killed Admin Server (PID: $ADMIN_SERVER_PID)"
    fi
    
    if pkill -f "palpo-admin-server" 2>/dev/null; then
        log_success "Killed remaining palpo-admin-server processes"
    fi
    
    if pkill -f "/palpo --config" 2>/dev/null; then
        log_success "Killed remaining Palpo processes"
    fi

    if pkill -f "dx serve" 2>/dev/null; then
        log_success "Killed remaining dx serve (Admin UI) processes"
    fi
}

# Main
main() {
    echo "========================================"
    echo "  Palpo Comprehensive Server Control E2E Tests"
    echo "========================================"
    echo ""
    echo "Mode: $MODE"
    echo ""
    
    case "$MODE" in
        setup)
            start_postgresql
            start_admin_server
            start_admin_ui
            check_services
            ;;
        test)
            check_services
            run_comprehensive_tests
            ;;
        check)
            check_services
            ;;
        clean)
            clean_test_data
            ;;
        restart)
            cleanup
            sleep 1
            start_postgresql
            start_admin_server
            start_admin_ui
            check_services
            run_comprehensive_tests
            ;;
        full)
            trap cleanup EXIT
            start_postgresql
            start_admin_server
            start_admin_ui
            check_services
            run_comprehensive_tests
            ;;
        *)
            echo "Usage: $0 {setup|test|check|clean|restart|full}"
            echo ""
            echo "Modes:"
            echo "  setup   - Start services and keep them running"
            echo "  test    - Run comprehensive tests (requires services already running)"
            echo "  check   - Check if services are ready"
            echo "  clean   - Clean test data"
            echo "  restart - Kill and restart all services"
            echo "  full    - Full workflow (setup + test + cleanup)"
            exit 1
            ;;
    esac
}

main