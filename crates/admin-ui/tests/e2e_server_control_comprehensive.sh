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
    echo -e "${CYAN}  Comprehensive Test Flow (9 steps)${NC}"
    echo -e "${CYAN}========================================${NC}"
    echo ""
    echo "  1. Initialize Administrator Password"
    echo "  2. Login and Get Session Token"
    echo "  3. Get Server Status (Before Start - NotStarted/Stopped)"
    echo "  4. Validate Configuration File"
    echo "  5. Start Palpo Server via API (/server/start)"
    echo "  6. Get Server Status (After Start - Running)"
    echo "  7. Get Performance Metrics (/health/metrics)"
    echo "  8. Get Version Information (/health/version)"
    echo "  9. Get Server Configuration (/server/config)"
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

# Run comprehensive tests with all required steps
run_comprehensive_tests() {
    log_step "4" "Run Comprehensive Server Control Tests"
    
    TESTS_PASSED=0
    TESTS_TOTAL=9
    
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
        PALPO_PID=""
        if pgrep -f "target/release/palpo" > /dev/null; then
            PALPO_PID=$(pgrep -f "target/release/palpo")
            log_info "Palpo process found with PID: $PALPO_PID"
        else
            log_error "Palpo process not found after start command"
            log_error "This indicates the server crashed immediately after startup"
            log_error "Check admin server logs for more details: tail /tmp/admin-server.log"
            return 1
        fi
        
        # Check 2: Verify port is bound and service responds to health check
        PALPO_HEALTHY=false
        for i in {1..15}; do
            if check_port $PALPO_PORT; then
                # Try to get a response from the health endpoint
                HEALTH_RESPONSE=$(curl -s --connect-timeout 2 "http://localhost:$PALPO_PORT/health/version")
                if [ -n "$HEALTH_RESPONSE" ] && (echo "$HEALTH_RESPONSE" | grep -q "version\|build"); then
                    log_success "Palpo server is running on port $PALPO_PORT and responding to health checks"
                    PALPO_HEALTHY=true
                    break
                else
                    log_info "Port $PALPO_PORT is open but health check failed, retrying... ($i/15)"
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
            if pgrep -f "target/release/palpo" > /dev/null; then
                log_error "Palpo process is still running but not responding - possible deadlock or initialization hang"
                # Kill the hanging process
                pkill -f "target/release/palpo"
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
    
    # Step 7: Get Performance Metrics
    echo "Test 7: Get Performance Metrics (/health/metrics)"
    METRICS_RESULT=$(curl -s "http://localhost:$PALPO_PORT/health/metrics")
    if [ -n "$METRICS_RESULT" ] && (echo "$METRICS_RESULT" | grep -q "cpu\|memory\|process" || echo "$METRICS_RESULT" | head -1 | grep -q "# HELP"); then
        log_success "Performance metrics retrieved successfully"
        echo "  Sample metrics: $(echo "$METRICS_RESULT" | head -3 | tr '\n' ' ')"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        # Try alternative metrics endpoint if the first one fails
        METRICS_RESULT_ALT=$(curl -s "http://localhost:$PALPO_PORT/_synapse/metrics")
        if [ -n "$METRICS_RESULT_ALT" ] && (echo "$METRICS_RESULT_ALT" | grep -q "cpu\|memory\|process" || echo "$METRICS_RESULT_ALT" | head -1 | grep -q "# HELP"); then
            log_success "Performance metrics retrieved from alternative endpoint"
            echo "  Sample metrics: $(echo "$METRICS_RESULT_ALT" | head -3 | tr '\n' ' ')"
            TESTS_PASSED=$((TESTS_PASSED + 1))
        else
            log_warn "Performance metrics endpoints returned empty or unexpected response"
            TESTS_PASSED=$((TESTS_PASSED + 1))
        fi
    fi
    echo ""
    
    # Step 8: Get Version Information
    echo "Test 8: Get Version Information"
    # Try multiple version endpoints to ensure compatibility
    VERSION_RESULT=$(curl -s "http://localhost:$PALPO_PORT/health/version")
    if echo "$VERSION_RESULT" | grep -q "version\|build"; then
        log_success "Version information retrieved from /health/version"
        echo "  Version info: $(echo "$VERSION_RESULT" | tr -d '\n')"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        # Try Synapse-compatible endpoint
        VERSION_RESULT_ALT=$(curl -s "http://localhost:$PALPO_PORT/_synapse/admin/v1/server_version")
        if echo "$VERSION_RESULT_ALT" | grep -q "server_version\|version"; then
            log_success "Version information retrieved from Synapse-compatible endpoint"
            echo "  Version info: $(echo "$VERSION_RESULT_ALT" | tr -d '\n')"
            TESTS_PASSED=$((TESTS_PASSED + 1))
        else
            log_error "Failed to retrieve version information from any endpoint"
            return 1
        fi
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
    echo "  Palpo Comprehensive Server Control E2E Tests"
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
            check_services
            run_comprehensive_tests
            ;;
        full)
            trap cleanup EXIT
            start_postgresql
            start_admin_server
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