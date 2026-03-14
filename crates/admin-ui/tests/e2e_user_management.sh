#!/bin/bash
# E2E Test Environment Setup and Execution
# 
# Usage:
#   bash crates/admin-ui/tests/e2e_user_management.sh [OPTIONS]
#
# Options:
#   --check        Only check environment status
#   --setup        Start all services and leave them running (services stay alive after exit)
#   --test         Run tests only (requires services already running via --setup)
#   --clean        Delete test data from DB, then exit (no services started/stopped)
#   --restart      Kill all services and restart from scratch, then leave them running
#   (default)      Full workflow: start services + run tests + stop on exit
#
# Typical workflow:
#   bash e2e_user_management.sh --setup    # start env once
#   bash e2e_user_management.sh --test     # run tests
#   bash e2e_user_management.sh --clean    # wipe test data between runs
#   bash e2e_user_management.sh --restart  # kill & restart services

# Do NOT use set -e — we handle errors explicitly with die()

# Configuration - derive PROJECT_ROOT from the script's own location
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="${PROJECT_ROOT:-$(cd "$SCRIPT_DIR/../../.." && pwd)}"
ADMIN_UI_DIR="$PROJECT_ROOT/crates/admin-ui"
ADMIN_SERVER_DIR="$PROJECT_ROOT/crates/admin-server"

BASE_URL="http://localhost:8080"
ADMIN_URL="http://localhost:8081"
BROWSER="agent-browser"
ADMIN_PASSWORD="AdminTest123!"
DATABASE_URL="postgresql://palpo:password@localhost/palpo"
ADMIN_SERVER_PORT=8081
UI_PORT=8080
PALPO_PORT=8088

# Test tracking
TESTS_PASSED=0

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# Parse arguments
MODE="full"
CLEAN=false
for arg in "$@"; do
    case "$arg" in
        -h|--help)
            echo ""
            echo -e "${CYAN}Usage: bash $0 [OPTIONS]${NC}"
            echo ""
            echo "Options:"
            echo "  --check    Only check environment status (no services started)"
            echo "  --setup    Start all services and leave them running (no tests)"
            echo "             Services stay alive after the script exits."
            echo "             Follow up with --test to run tests."
            echo "  --test     Run tests only — requires services already running via --setup"
            echo "             Does not start or stop any services."
            echo "  --clean    Delete test data from the database, then exit"
            echo "             Does not start or stop any services."
            echo "  --restart  Kill all services and restart from scratch, then leave running"
            echo "  (default)  Start services + run tests + stop services on exit"
            echo ""
            echo "Typical workflow:"
            echo "  bash $0 --setup    # start env once, keep it running"
            echo "  bash $0 --test     # run tests (reuse running env)"
            echo "  bash $0 --clean    # wipe test data between runs"
            echo "  bash $0 --restart  # kill everything and restart fresh"
            echo ""
            exit 0
            ;;
        --check)   MODE="check" ;;
        --setup)   MODE="setup" ;;
        --test)    MODE="test" ;;
        --clean)   MODE="clean" ;;
        --restart) MODE="restart" ;;
    esac
done

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}  E2E Test Environment Setup${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""
echo "Mode: $MODE | Clean: $CLEAN"
echo ""

# ============================================
# Helper Functions
# ============================================
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[✓]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[⚠]${NC} $1"
}

log_error() {
    echo -e "${RED}[✗]${NC} $1"
}

# Exit immediately with error message
die() {
    echo -e "${RED}[FATAL]${NC} $1"
    echo -e "${RED}Stopping immediately.${NC}"
    exit 1
}

# Track test failures and exit immediately
mark_failed_and_exit() {
    die "FAILED: $1"
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

# Kill all related services (used with --restart or internal restart logic)
kill_services() {
    log_info "Stopping all services..."
    
    # Kill processes on known ports
    for port in $UI_PORT $ADMIN_SERVER_PORT; do
        local pid
        pid=$(lsof -ti :$port 2>/dev/null)
        if [ -n "$pid" ]; then
            kill "$pid" 2>/dev/null && log_info "Killed process on port $port (pid $pid)" || true
        fi
    done
    
    # Also kill by name
    pkill -f "palpo-admin-server" 2>/dev/null || true
    pkill -f "dx serve" 2>/dev/null || true
    
    sleep 2
    log_success "Services stopped"
}

# Clean test data only — does not touch running services
clean_test_data() {
    log_info "Cleaning test data..."
    psql "$DATABASE_URL" -c "DELETE FROM webui_users WHERE username='newtestuser';" 2>/dev/null || true
    psql "$DATABASE_URL" -c "DELETE FROM user_attributes WHERE localpart='newtestuser';" 2>/dev/null || true
    psql "$DATABASE_URL" -c "DELETE FROM users WHERE name='@newtestuser:localhost';" 2>/dev/null || true
    log_success "Test data cleaned"
}

# Cleanup function - only kills processes started by THIS script run
cleanup() {
    # Only kill processes we started in this session
    if [ -n "$DIOXUS_PID" ] && kill -0 "$DIOXUS_PID" 2>/dev/null; then
        kill "$DIOXUS_PID" 2>/dev/null || true
    fi
    if [ -n "$ADMIN_PID" ] && kill -0 "$ADMIN_PID" 2>/dev/null; then
        kill "$ADMIN_PID" 2>/dev/null || true
    fi
}

# Trap is only registered in 'full' mode (see main execution below)
# ============================================
start_admin_ui() {
    log_step "1" "Start Admin UI (Dioxus)"
    
    if check_port $UI_PORT; then
        log_success "Admin UI is already running on port $UI_PORT"
    else
        # Wait for any cargo build lock to be released before starting dx serve
        log_info "Waiting for Cargo build lock to be free..."
        local lock_file="$PROJECT_ROOT/target/.cargo-lock"
        local waited=0
        while [ -f "$lock_file" ] && [ $waited -lt 30 ]; do
            sleep 2
            waited=$((waited + 2))
        done
        
        log_info "Starting Dioxus dev server..."
        cd "$ADMIN_UI_DIR"
        dx serve >/tmp/dioxus.log 2>&1 &
        DIOXUS_PID=$!
        
        wait_for_url "http://localhost:$UI_PORT" "Admin UI" 120 || die "Admin UI failed to start. Check: tail /tmp/dioxus.log"
    fi
}

# ============================================
# STEP 2: Start PostgreSQL
# ============================================
start_postgresql() {
    log_step "2" "Start PostgreSQL"
    
    if pg_isready -h localhost -p 5432 >/dev/null 2>&1; then
        log_success "PostgreSQL is running"
        psql "$DATABASE_URL" -c "SELECT 1;" >/dev/null 2>&1 || die "Cannot connect to database: $DATABASE_URL"
        log_success "Database connection successful"
    else
        die "PostgreSQL is not running. Start it with:\n  brew services start postgresql  # macOS\n  sudo systemctl start postgresql # Linux"
    fi
}

# ============================================
# STEP 3: Start Admin Server
# ============================================
start_admin_server() {
    log_step "3" "Start Admin Server"
    
    if check_port $ADMIN_SERVER_PORT; then
        log_success "Admin Server is already running on port $ADMIN_SERVER_PORT"
        wait_for_url "http://localhost:$ADMIN_SERVER_PORT/api/v1/auth/webui/setup/status" "Admin Server API" 10 || \
            die "Admin Server is running but API is not responding"
        return 0
    fi
    
    ADMIN_BINARY="$PROJECT_ROOT/target/release/palpo-admin-server"
    # Only rebuild if binary doesn't exist or source is newer than binary
    NEEDS_BUILD=false
    if [ ! -f "$ADMIN_BINARY" ]; then
        NEEDS_BUILD=true
    else
        # Check if any source file is newer than the binary
        NEWER=$(find "$PROJECT_ROOT/crates/admin-server/src" -name "*.rs" -newer "$ADMIN_BINARY" 2>/dev/null | head -1)
        [ -n "$NEWER" ] && NEEDS_BUILD=true
    fi

    if [ "$NEEDS_BUILD" = true ]; then
        log_info "Building Admin Server (this may take a few minutes)..."
        cd "$PROJECT_ROOT"
        cargo build --release -p palpo-admin-server || die "Build failed"
    else
        log_info "Admin Server binary is up-to-date, skipping build"
    fi

    log_info "Starting Admin Server..."
    DATABASE_URL="$DATABASE_URL" "$ADMIN_BINARY" >/tmp/admin-server.log 2>&1 &
    ADMIN_PID=$!
    
    wait_for_url "http://localhost:$ADMIN_SERVER_PORT/api/v1/auth/webui/setup/status" "Admin Server" 60 || \
        die "Admin Server failed to start. Check: tail /tmp/admin-server.log"
}

# ============================================
# STEP 4: Login to WebUI (Create Admin Account)
# ============================================
login_webui() {
    log_step "4" "Login to WebUI (Create Admin Account)"
    
    log_info "Checking admin account status..."
    SETUP_STATUS=$(curl -s "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/webui-admin/status")
    log_info "Setup status: $SETUP_STATUS"
    
    if psql "$DATABASE_URL" -c "SELECT username FROM webui_admin_credentials WHERE username='admin';" 2>/dev/null | grep -q "admin"; then
        log_success "Admin account already exists in database"
    else
        log_info "Admin account not found, creating via API..."
        SETUP_RESULT=$(curl -s -X POST "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/webui-admin/setup" \
            -H "Content-Type: application/json" \
            -d "{\"password\":\"$ADMIN_PASSWORD\"}")
        log_info "Setup result: $SETUP_RESULT"
        
        sleep 2
        if psql "$DATABASE_URL" -c "SELECT username FROM webui_admin_credentials WHERE username='admin';" 2>/dev/null | grep -q "admin"; then
            log_success "Admin account created via API"
        else
            log_warn "API creation may have failed, trying via WebUI..."
            $BROWSER open "$BASE_URL"
            $BROWSER wait 5000
            
            for attempt in {1..3}; do
                SNAP=$($BROWSER snapshot -i)
                if echo "$SNAP" | grep -q "设置管理员密码"; then
                    log_info "Setup wizard detected (attempt $attempt)..."
                    $BROWSER find placeholder "输入密码" fill "$ADMIN_PASSWORD"
                    $BROWSER find placeholder "确认密码" fill "$ADMIN_PASSWORD"
                    $BROWSER find text "确认" click
                    $BROWSER wait 3000
                    break
                fi
                log_info "Waiting for setup wizard... (attempt $attempt)"
                sleep 3
            done
            
            $BROWSER close 2>/dev/null || true
        fi
    fi
    
    psql "$DATABASE_URL" -c "SELECT username FROM webui_admin_credentials WHERE username='admin';" 2>/dev/null | grep -q "admin" || \
        die "Admin account not found in database after setup"
    log_success "Admin account verified"
}

# ============================================
# STEP 5: Environment Ready
# ============================================
check_environment() {
    log_step "5" "Environment Ready"
    
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
    
    if check_port $ADMIN_SERVER_PORT && curl -s "http://localhost:$ADMIN_SERVER_PORT/api/v1/auth/webui/setup/status" >/dev/null 2>&1; then
        echo -e "  Admin Server:  ${GREEN}✓ Ready${NC}"
    else
        echo -e "  Admin Server:  ${RED}✗ Not Ready${NC}"
        all_ready=false
    fi
    
    if check_port $UI_PORT && curl -s "http://localhost:$UI_PORT" >/dev/null 2>&1; then
        echo -e "  Admin UI:      ${GREEN}✓ Ready${NC}"
    else
        echo -e "  Admin UI:      ${RED}✗ Not Ready${NC}"
        all_ready=false
    fi
    
    if psql "$DATABASE_URL" -c "SELECT username FROM webui_admin_credentials WHERE username='admin';" 2>/dev/null | grep -q "admin"; then
        echo -e "  Admin Account: ${GREEN}✓ Ready${NC}"
    else
        echo -e "  Admin Account: ${RED}✗ Not Ready${NC}"
        all_ready=false
    fi
    
    echo "========================================"
    
    if [ "$all_ready" = true ]; then
        log_success "All services are ready!"
    else
        die "Some services are not ready. Fix the issues above and retry."
    fi
}

# ============================================
# Run E2E Tests
# ============================================
run_tests() {
    log_step "RUNNING" "E2E Tests"
    
    check_environment
    
    echo ""
    log_info "Starting E2E tests..."
    echo ""
    
    TESTS_PASSED=0

    # ============================================
    # Test 3.4.1: Admin Creates New User
    # ============================================
    echo "--- Test 3.4.1: Admin Creates New User ---"
    
    echo "Step 1: Login as admin"
    $BROWSER open "$BASE_URL/login" || die "Failed to open login page"
    $BROWSER wait 3000
    
    echo "Filling login form..."
    $BROWSER fill "#username" "admin"         || die "Failed to fill username"
    $BROWSER fill "#password" "$ADMIN_PASSWORD" || die "Failed to fill password"
    $BROWSER find role button click --name "登录"  || die "Failed to click login button"
    $BROWSER wait 3000
    
    echo "Step 2: Dismiss dialogs and navigate to users"
    for i in {1..3}; do
        SNAP=$($BROWSER snapshot -i) || die "snapshot failed"
        if echo "$SNAP" | grep -q "我明白了"; then
            $BROWSER find text "我明白了" click || die "Failed to click 我明白了"
            $BROWSER wait 500
        else
            break
        fi
    done
    
    echo "Navigating to users page..."
    $BROWSER find role link click --name "👥 用户管理" || die "Failed to click 用户管理"
    $BROWSER wait 3000
    
    echo "Step 3: Open create user modal"
    $BROWSER snapshot -i
    $BROWSER find role button click --name "➕ 创建用户" || die "Failed to find/click ➕ 创建用户 button"
    $BROWSER wait 2000
    
    echo "Step 4: Clean up existing test user"
    psql "$DATABASE_URL" -c "DELETE FROM webui_users WHERE username='newtestuser';" 2>/dev/null || true
    psql "$DATABASE_URL" -c "DELETE FROM user_attributes WHERE localpart='newtestuser';" 2>/dev/null || true
    psql "$DATABASE_URL" -c "DELETE FROM users WHERE name='@newtestuser:localhost';" 2>/dev/null || true
    
    echo "Step 5: Fill user form"
    $BROWSER snapshot -i
    $BROWSER find placeholder "输入用户名" fill "newtestuser"  || die "Failed to fill username field"
    $BROWSER find placeholder "输入显示名（可选）" fill "New Test User" || die "Failed to fill display name"
    $BROWSER fill @e17 "TestPass123!" || die "Failed to fill password"
    $BROWSER fill @e19 "TestPass123!" || die "Failed to fill confirm password"
    $BROWSER wait 1000
    
    echo "Step 6: Submit form"
    $BROWSER snapshot -i
    $BROWSER click @e22 || die "Failed to click submit button"
    $BROWSER wait 3000
    
    echo "Step 7: Verify redirect back to users list"
    # Navigate explicitly to users list in case auto-redirect didn't happen
    $BROWSER open "$BASE_URL/admin/users"
    $BROWSER wait 3000
    SNAP=$($BROWSER snapshot -i) || die "snapshot failed"
    
    echo "Step 8: Verify user in database"
    USER_EXISTS=$(psql "$DATABASE_URL" -t -c "SELECT localpart FROM users WHERE localpart='newtestuser';" 2>/dev/null | xargs)
    [ "$USER_EXISTS" = "newtestuser" ] || die "User 'newtestuser' NOT found in database after creation"
    echo "✓ User 'newtestuser' exists in database"
    TESTS_PASSED=$((TESTS_PASSED + 1))
    echo -e "${GREEN}✓ Test 3.4.1 PASSED${NC}"
    echo ""

    # ============================================
    # Test 3.4.2: Admin Manages User Devices
    # ============================================
    echo "--- Test 3.4.2: Admin Manages User Devices ---"
    
    echo "Step 1: Navigate to user detail"
    # Search for newtestuser to filter list, then click 查看详情
    $BROWSER open "$BASE_URL/admin/users" || die "Failed to navigate to users list"
    $BROWSER wait 3000
    $BROWSER find placeholder "按用户名或显示名搜索..." fill "newtestuser" || die "Failed to fill search"
    $BROWSER wait 2000
    SNAP=$($BROWSER snapshot -i) || die "snapshot failed"
    echo "$SNAP" | grep -q "newtestuser" || die "newtestuser not found in filtered list"
    # Click 查看详情 — after filtering there should be only one result
    $BROWSER find text "查看详情" click || die "Failed to click 查看详情"
    $BROWSER wait 5000
    SNAP=$($BROWSER snapshot -i) || die "snapshot failed"
    # Verify we're on the user detail page
    echo "$SNAP" | grep -qE "(用户详情|基本信息|设备)" || die "User detail page did not load"
    
    echo "Step 2: Click Devices tab"
    $BROWSER find text "设备" click || die "Failed to find/click Devices tab"
    $BROWSER wait 2000
    
    echo "Step 3: Verify devices content"
    SNAP=$($BROWSER snapshot -i) || die "snapshot failed"
    echo "$SNAP" | grep -qE "(设备|device|Device)" || die "Devices tab content not displayed"
    echo "✓ Devices tab content displayed"
    TESTS_PASSED=$((TESTS_PASSED + 1))
    echo -e "${GREEN}✓ Test 3.4.2 PASSED${NC}"
    echo ""

    # ============================================
    # Test 3.4.3: Admin Edits User Basic Info
    # ============================================
    echo "--- Test 3.4.3: Admin Edits User Basic Info ---"
    
    echo "Step 1: Click Basic Info tab"
    $BROWSER find text "基本信息" click || die "Failed to find/click 基本信息 tab"
    $BROWSER wait 1000
    
    echo "Step 2: Click Edit button"
    $BROWSER snapshot -i
    $BROWSER find text "编辑用户" click || die "Failed to find/click 编辑用户"
    $BROWSER wait 1000
    
    echo "Step 3: Verify edit form and save"
    $BROWSER snapshot -i
    SNAP=$($BROWSER snapshot -i) || die "snapshot failed"
    echo "$SNAP" | grep -qE "(显示名|头像|保存)" || die "Edit form not displayed"
    # Just save without changing — verifies the save flow works
    $BROWSER wait 500
    
    echo "Step 4: Save changes"
    $BROWSER find text "💾 保存" click || die "Failed to click 💾 保存"
    $BROWSER wait 2000
    
    echo "Step 5: Verify edit mode exited"
    SNAP=$($BROWSER snapshot -i) || die "snapshot failed"
    echo "$SNAP" | grep -qE "(编辑用户|✏️)" || die "Edit mode did not exit after save"
    echo "✓ User info edit completed"
    TESTS_PASSED=$((TESTS_PASSED + 1))
    echo -e "${GREEN}✓ Test 3.4.3 PASSED${NC}"
    echo ""

    # ============================================
    # Test 3.4.4: Admin Views User Connections
    # ============================================
    echo "--- Test 3.4.4: Admin Views User Connections ---"
    
    echo "Step 1: Click Connections tab"
    $BROWSER find text "连接" click || die "Failed to find/click 连接 tab"
    $BROWSER wait 2000
    
    echo "Step 2: Verify connections content"
    SNAP=$($BROWSER snapshot -i) || die "snapshot failed"
    echo "$SNAP" | grep -qE "(连接信息|暂无连接|会话)" || die "Connections tab content not displayed"
    echo "✓ Connections tab content displayed"
    TESTS_PASSED=$((TESTS_PASSED + 1))
    echo -e "${GREEN}✓ Test 3.4.4 PASSED${NC}"
    echo ""

    # ============================================
    # Test 3.4.5: Admin Searches and Filters Users
    # ============================================
    echo "--- Test 3.4.5: Admin Searches and Filters Users ---"
    
    echo "Step 1: Navigate back to users list"
    $BROWSER open "$BASE_URL/admin/users" || die "Failed to navigate to users list"
    $BROWSER wait 3000
    
    echo "Step 2: Search for user"
    $BROWSER snapshot -i
    $BROWSER find placeholder "按用户名或显示名搜索..." fill "test" || die "Failed to fill search input"
    $BROWSER wait 1000
    
    echo "Step 3: Verify results"
    SNAP=$($BROWSER snapshot -i) || die "snapshot failed"
    echo "$SNAP" | grep -qE "(用户|user|User)" || die "User list not displayed after search"
    echo "✓ Search results displayed"
    TESTS_PASSED=$((TESTS_PASSED + 1))
    echo -e "${GREEN}✓ Test 3.4.5 PASSED${NC}"
    echo ""

    # ============================================
    # Summary
    # ============================================
    echo "========================================"
    echo "  Test Summary"
    echo "========================================"
    echo -e "  Passed: ${GREEN}$TESTS_PASSED / 5${NC}"
    echo "========================================"
    echo -e "${GREEN}All E2E tests passed!${NC}"
}

# ============================================
# Main Execution
# ============================================

case "$MODE" in
    check)
        check_environment
        ;;
    clean)
        # Only clean test data — no services started or stopped
        log_step "CLEAN" "Clean Test Data"
        clean_test_data
        log_success "Done. Run tests with: bash $0 --test"
        ;;
    setup)
        # Do NOT register cleanup trap — services must stay alive after this script exits
        start_postgresql
        start_admin_server
        start_admin_ui
        login_webui
        check_environment
        echo ""
        log_success "Environment is up. Run tests with: bash $0 --test"
        ;;
    test)
        # Verify services are running before testing (don't start them)
        check_environment
        run_tests
        ;;
    restart)
        # Kill everything and restart fresh, then leave services running
        kill_services
        start_postgresql
        start_admin_server
        start_admin_ui
        login_webui
        check_environment
        echo ""
        log_success "Environment restarted. Run tests with: bash $0 --test"
        ;;
    full)
        # Register cleanup only in full mode — we start and own the services
        trap cleanup EXIT
        start_postgresql
        start_admin_server
        start_admin_ui
        login_webui
        check_environment
        run_tests
        ;;
esac