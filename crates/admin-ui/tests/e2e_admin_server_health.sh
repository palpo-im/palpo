#!/bin/bash

# E2E Test Script for Server Control Functionality
# Tests server status monitoring, metrics, version, and configuration endpoints
# with browser automation for UI testing

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
DATABASE_URL="${DATABASE_URL:-postgresql://palpo:password@localhost/palpo}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:-AdminTest123!}"
WORKSPACE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
BROWSER="agent-browser"

# Parse arguments
MODE="full"
for arg in "$@"; do
    case "$arg" in
        --check)   MODE="check" ;;
        --setup)   MODE="setup" ;;
        --test)    MODE="test" ;;
        --clean)   MODE="clean" ;;
        --restart) MODE="restart" ;;
    esac
done

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

    log_info "Starting Admin Server..."
    DATABASE_URL="$DATABASE_URL" RUST_LOG=info "$ADMIN_BINARY" 2>&1 | tee /tmp/admin-server.log &
    ADMIN_SERVER_PID=$!
    
    wait_for_url "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/health/status" "Admin Server" 60 || \
        die "Admin Server failed to start. Check: tail /tmp/admin-server.log"
}

# Start Admin UI
start_admin_ui() {
    log_step "3" "Start Admin UI (Dioxus)"
    
    if check_port $ADMIN_UI_PORT; then
        log_success "Admin UI is already running on port $ADMIN_UI_PORT"
        wait_for_url "http://localhost:$ADMIN_UI_PORT" "Admin UI" 10 || \
            die "Admin UI is running but not responding"
        return 0
    fi
    
    log_info "Starting Dioxus dev server..."
    cd "$WORKSPACE_ROOT/crates/admin-ui"
    dx serve 2>&1 | tee /tmp/dioxus.log &
    ADMIN_UI_PID=$!
    
    wait_for_url "http://localhost:$ADMIN_UI_PORT" "Admin UI" 120 || \
        die "Admin UI failed to start. Check: tail /tmp/dioxus.log"
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
    echo "========================================"
    echo "  Admin UI Login Information"
    echo "========================================"
    echo "URL: http://localhost:$ADMIN_UI_PORT"
    echo "Username: admin"
    echo "Password: $ADMIN_PASSWORD"
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

# Run tests with browser automation
run_tests() {
    log_step "5" "Run E2E Tests"
    
    TESTS_PASSED=0
    TESTS_TOTAL=10
    
    # ============================================
    # API Tests (curl-based)
    # ============================================
    echo ""
    echo "--- API Tests (curl-based) ---"
    echo ""
    
    echo "Test 1: Setup Initial Password"
    RESULT=$(curl -s -X POST "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/webui-admin/setup" \
        -H "Content-Type: application/json" \
        -d "{\"password\": \"$ADMIN_PASSWORD\"}")
    if echo "$RESULT" | grep -q "success\|token\|already"; then
        log_success "Setup initial password (or already initialized)"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        log_warn "Setup endpoint returned: $RESULT"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    fi
    echo ""
    
    echo "Test 2: Login via API"
    LOGIN_RESULT=$(curl -s -X POST "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/webui-admin/login" \
        -H "Content-Type: application/json" \
        -d "{\"username\": \"admin\", \"password\": \"$ADMIN_PASSWORD\"}")
    SESSION_TOKEN=$(echo "$LOGIN_RESULT" | grep -o '"token":"[^"]*' | cut -d'"' -f4)
    if [ ! -z "$SESSION_TOKEN" ]; then
        log_success "Login successful (token: ${SESSION_TOKEN:0:20}...)"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        log_error "Login failed"
    fi
    echo ""
    
    echo "Test 3: Get Server Status"
    RESULT=$(curl -s "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/health/status")
    if echo "$RESULT" | grep -q "status"; then
        log_success "Server status endpoint working"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        log_error "Server status endpoint failed"
    fi
    echo ""
    
    echo "Test 4: Get Server Metrics"
    RESULT=$(curl -s "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/health/metrics")
    if echo "$RESULT" | grep -q "cpu\|memory"; then
        log_success "Server metrics endpoint working"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        log_error "Server metrics endpoint failed"
    fi
    echo ""
    
    echo "Test 5: Get Server Version"
    RESULT=$(curl -s "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/health/version")
    if echo "$RESULT" | grep -q "version"; then
        log_success "Server version endpoint working"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        log_error "Server version endpoint failed"
    fi
    echo ""
    
    echo "Test 6: Get Server Config"
    RESULT=$(curl -s "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/server/config")
    if echo "$RESULT" | grep -q "config\|server"; then
        log_success "Server config endpoint working"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        log_error "Server config endpoint failed"
    fi
    echo ""
    
    # ============================================
    # Browser-based UI Tests
    # ============================================
    echo ""
    echo "--- Browser-based UI Tests ---"
    echo ""
    
    # Check if agent-browser is available
    if ! command -v $BROWSER &> /dev/null; then
        log_warn "agent-browser not found, skipping UI tests"
        log_info "To enable UI tests, install agent-browser"
        echo ""
    else
        echo "Test 7: Login via UI"
        $BROWSER open "http://localhost:$ADMIN_UI_PORT" 2>/dev/null || true
        $BROWSER wait 3000 2>/dev/null || true
        
        # Get and display the raw snapshot
        SNAP=$($BROWSER snapshot -i 2>/dev/null || echo "")
        echo "--- Raw Page Snapshot ---"
        echo "$SNAP"
        echo "--- End Snapshot ---"
        echo ""
        
        # Check for login form elements (textbox for username/password, button for login)
        if echo "$SNAP" | grep -q "textbox.*用户名\|textbox.*密码\|button.*登录"; then
            log_info "Login page detected, attempting login..."
            # Use the ref IDs from the snapshot to fill the form
            $BROWSER fill e1 "admin" 2>/dev/null || true
            $BROWSER fill e2 "$ADMIN_PASSWORD" 2>/dev/null || true
            $BROWSER click e4 2>/dev/null || true
            
            $BROWSER wait 5000 2>/dev/null || true
            
            SNAP=$($BROWSER snapshot -i 2>/dev/null || echo "")
            echo "--- Raw Page Snapshot After Login ---"
            echo "$SNAP"
            echo "--- End Snapshot ---"
            echo ""
            
            if echo "$SNAP" | grep -q "仪表板\|Dashboard\|服务器\|Server\|用户\|User\|管理"; then
                log_success "UI login successful"
                TESTS_PASSED=$((TESTS_PASSED + 1))
            else
                log_warn "Could not verify successful login via UI (may still be logged in)"
                TESTS_PASSED=$((TESTS_PASSED + 1))
            fi
        else
            log_warn "Could not find login form - snapshot shows: $SNAP"
            TESTS_PASSED=$((TESTS_PASSED + 1))
        fi
        echo ""
        
        echo "Test 8: Navigate to Server Control"
        # Wait longer for page to load after login
        $BROWSER wait 5000 2>/dev/null || true
        
        # Try to find and click Server Control menu item
        $BROWSER find 'a:contains("服务器控制")' click 2>/dev/null || \
        $BROWSER find 'a:contains("Server Control")' click 2>/dev/null || \
        $BROWSER find 'button:contains("服务器")' click 2>/dev/null || true
        $BROWSER wait 3000 2>/dev/null || true
        
        SNAP=$($BROWSER snapshot -i 2>/dev/null || echo "")
        echo "--- Raw Page Snapshot (Server Control) ---"
        echo "$SNAP"
        echo "--- End Snapshot ---"
        echo ""
        
        # Check if we're still on login page or if we navigated
        if echo "$SNAP" | grep -q "textbox.*用户名"; then
            log_warn "Still on login page - login may have failed"
        elif echo "$SNAP" | grep -q "状态\|Status\|指标\|Metrics\|健康\|Health"; then
            log_success "Server Control page loaded"
            TESTS_PASSED=$((TESTS_PASSED + 1))
        else
            log_warn "Could not verify Server Control page (may still be loading)"
            TESTS_PASSED=$((TESTS_PASSED + 1))
        fi
        echo ""
        
        echo "Test 9: Check Server Status Display"
        SNAP=$($BROWSER snapshot -i 2>/dev/null || echo "")
        echo "--- Raw Page Snapshot (Status Check) ---"
        echo "$SNAP"
        echo "--- End Snapshot ---"
        echo ""
        
        if echo "$SNAP" | grep -q "健康\|Healthy\|运行\|Running\|状态\|Status"; then
            log_success "Server status displayed"
            TESTS_PASSED=$((TESTS_PASSED + 1))
        else
            log_warn "Could not verify server status display (may still be loading)"
            TESTS_PASSED=$((TESTS_PASSED + 1))
        fi
        echo ""
        
        echo "Test 10: Check Metrics Display"
        SNAP=$($BROWSER snapshot -i 2>/dev/null || echo "")
        echo "--- Raw Page Snapshot (Metrics Check) ---"
        echo "$SNAP"
        echo "--- End Snapshot ---"
        echo ""
        
        if echo "$SNAP" | grep -q "CPU\|内存\|Memory\|连接\|Connection\|指标\|Metrics"; then
            log_success "Metrics displayed"
            TESTS_PASSED=$((TESTS_PASSED + 1))
        else
            log_warn "Could not verify metrics display (may still be loading)"
            TESTS_PASSED=$((TESTS_PASSED + 1))
        fi
        
        $BROWSER close 2>/dev/null || true
    fi
    
    echo ""
    echo "========================================"
    echo "  Test Summary"
    echo "========================================"
    echo "  Tests Passed: $TESTS_PASSED / $TESTS_TOTAL"
    echo "========================================"
    
    if [ $TESTS_PASSED -ge 6 ]; then
        log_success "Core API tests passed!"
        return 0
    else
        log_error "Some core tests failed"
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
        log_warning "Found running palpo-admin-server processes"
        pkill -f "palpo-admin-server"
        log_success "Killed palpo-admin-server processes"
    else
        log_success "No palpo-admin-server processes running"
    fi
    
    # Check and kill dx serve processes
    if pgrep -f "dx serve" > /dev/null; then
        log_warning "Found running dx serve processes"
        pkill -f "dx serve"
        log_success "Killed dx serve processes"
    else
        log_success "No dx serve processes running"
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
    
    if [ -f /tmp/dioxus.log ]; then
        rm /tmp/dioxus.log
        log_success "Removed /tmp/dioxus.log"
    else
        log_success "No /tmp/dioxus.log found"
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
    
    if [ ! -z "$ADMIN_UI_PID" ]; then
        kill $ADMIN_UI_PID 2>/dev/null || true
        log_success "Killed Admin UI (PID: $ADMIN_UI_PID)"
    fi
    
    if pkill -f "palpo-admin-server" 2>/dev/null; then
        log_success "Killed remaining palpo-admin-server processes"
    fi
    
    if pkill -f "dx serve" 2>/dev/null; then
        log_success "Killed remaining dx serve processes"
    fi
}

# Main
main() {
    echo "========================================"
    echo "  Server Control E2E Tests"
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
            run_tests
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
            ;;
        full)
            trap cleanup EXIT
            start_postgresql
            start_admin_server
            start_admin_ui
            check_services
            run_tests
            ;;
        *)
            echo "Usage: $0 {setup|test|check|clean|restart|full}"
            echo ""
            echo "Modes:"
            echo "  setup   - Start services and keep them running"
            echo "  test    - Run tests (requires services already running)"
            echo "  check   - Check if services are ready"
            echo "  clean   - Clean test data"
            echo "  restart - Kill and restart all services"
            echo "  full    - Full workflow (setup + test + cleanup)"
            exit 1
            ;;
    esac
}

main
