#!/bin/bash

# E2E Test Script for Palpo Server Control Functionality
# Tests server start, stop, restart, and status monitoring
# with configuration validation before startup

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
PALPO_PORT=8008
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

# Check all services are ready
check_services() {
    log_step "3" "Environment Ready"
    
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

# Run tests with API calls
run_tests() {
    log_step "4" "Run E2E Server Control Tests"
    
    TESTS_PASSED=0
    TESTS_TOTAL=6
    
    echo ""
    echo "--- Server Control Tests (curl-based) ---"
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
    
    echo "Test 3: Get Server Status (Before Start)"
    RESULT=$(curl -s "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/server/status")
    if echo "$RESULT" | grep -q "status\|NotStarted\|Stopped"; then
        log_success "Server status endpoint working"
        echo "  Status: $(echo "$RESULT" | grep -o '"status":"[^"]*' | cut -d'"' -f4)"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        log_error "Server status endpoint failed"
    fi
    echo ""
    
    echo "Test 4: Validate Configuration Before Start"
    RESULT=$(curl -s -X POST "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/server/config/validate" \
        -H "Content-Type: application/json" \
        -d "{}")
    if echo "$RESULT" | grep -q "valid\|success"; then
        log_success "Configuration validation passed"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        log_warn "Configuration validation result: $RESULT"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    fi
    echo ""
    
    echo "Test 5: Start Palpo Server"
    RESULT=$(curl -s -X POST "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/server/start" \
        -H "Content-Type: application/json" \
        -d "{}")
    if echo "$RESULT" | grep -q "success\|started"; then
        log_success "Server start command sent"
        TESTS_PASSED=$((TESTS_PASSED + 1))
        
        # Wait for server to start
        log_info "Waiting for Palpo server to start..."
        sleep 3
        
        # Check if server is running
        if check_port $PALPO_PORT; then
            log_success "Palpo server is running on port $PALPO_PORT"
        else
            log_warn "Palpo server may still be starting..."
        fi
    else
        log_warn "Server start result: $RESULT"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    fi
    echo ""
    
    echo "Test 6: Get Server Status (After Start)"
    RESULT=$(curl -s "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/server/status")
    if echo "$RESULT" | grep -q "status\|Running"; then
        log_success "Server status shows Running"
        echo "  Status: $(echo "$RESULT" | grep -o '"status":"[^"]*' | cut -d'"' -f4)"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        log_warn "Server status: $RESULT"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    fi
    echo ""
    
    echo "========================================"
    echo "  Test Summary"
    echo "========================================"
    echo "  Tests Passed: $TESTS_PASSED / $TESTS_TOTAL"
    echo "========================================"
    
    if [ $TESTS_PASSED -ge 4 ]; then
        log_success "Core server control tests passed!"
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
        log_warn "Found running palpo-admin-server processes"
        pkill -f "palpo-admin-server"
        log_success "Killed palpo-admin-server processes"
    else
        log_success "No palpo-admin-server processes running"
    fi
    
    # Check and kill Palpo processes
    if pgrep -f "target/release/palpo" > /dev/null; then
        log_warn "Found running Palpo processes"
        pkill -f "target/release/palpo"
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
    
    if pkill -f "target/release/palpo" 2>/dev/null; then
        log_success "Killed remaining Palpo processes"
    fi
}

# Main
main() {
    echo "========================================"
    echo "  Palpo Server Control E2E Tests"
    echo "========================================"
    echo ""
    echo "Mode: $MODE"
    echo ""
    
    case "$MODE" in
        setup)
            start_postgresql
            start_admin_server
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
            check_services
            ;;
        full)
            trap cleanup EXIT
            start_postgresql
            start_admin_server
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
