#!/bin/bash

# E2E Test Script for Server Control Functionality
# Tests server status monitoring, metrics, version, and configuration endpoints

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Configuration
ADMIN_SERVER_PORT=8081
ADMIN_UI_PORT=8080
DATABASE_URL="${DATABASE_URL:-postgresql://palpo:password@localhost/palpo}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:-AdminTest123!}"
WORKSPACE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# Logging functions
log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[✓]${NC} $1"
}

# Check if service is running
is_service_running() {
    local port=$1
    nc -z localhost "$port" 2>/dev/null && return 0 || return 1
}

# Wait for service to be ready
wait_for_service() {
    local port=$1
    local max_attempts=30
    local attempt=0
    
    while [ $attempt -lt $max_attempts ]; do
        if is_service_running "$port"; then
            return 0
        fi
        attempt=$((attempt + 1))
        sleep 1
    done
    
    return 1
}

# Start PostgreSQL
start_postgres() {
    log_info "Starting PostgreSQL..."
    if ! pg_isready -h localhost -p 5432 >/dev/null 2>&1; then
        brew services start postgresql 2>/dev/null || true
        sleep 2
    fi
    
    if pg_isready -h localhost -p 5432 >/dev/null 2>&1; then
        log_success "PostgreSQL is running"
    else
        log_error "Failed to start PostgreSQL"
        return 1
    fi
}

# Start Admin Server
start_admin_server() {
    log_info "Starting Admin Server..."
    
    if is_service_running $ADMIN_SERVER_PORT; then
        log_warning "Admin Server already running on port $ADMIN_SERVER_PORT"
        return 0
    fi
    
    cd "$WORKSPACE_ROOT/crates/admin-server"
    cargo build --release 2>&1 | tail -5
    
    RUST_LOG=info cargo run --release > /tmp/admin-server.log 2>&1 &
    ADMIN_SERVER_PID=$!
    
    if wait_for_service $ADMIN_SERVER_PORT; then
        log_success "Admin Server started (PID: $ADMIN_SERVER_PID)"
        return 0
    else
        log_error "Admin Server failed to start"
        cat /tmp/admin-server.log
        return 1
    fi
}

# Start Admin UI
start_admin_ui() {
    log_info "Starting Admin UI..."
    
    if is_service_running $ADMIN_UI_PORT; then
        log_warning "Admin UI already running on port $ADMIN_UI_PORT"
        return 0
    fi
    
    cd "$WORKSPACE_ROOT/crates/admin-ui"
    dx serve > /tmp/dioxus.log 2>&1 &
    ADMIN_UI_PID=$!
    
    if wait_for_service $ADMIN_UI_PORT; then
        log_success "Admin UI started (PID: $ADMIN_UI_PID)"
        return 0
    else
        log_error "Admin UI failed to start"
        cat /tmp/dioxus.log
        return 1
    fi
}

# Check all services are ready
check_services() {
    log_info "Checking services..."
    
    if ! pg_isready -h localhost -p 5432 >/dev/null 2>&1; then
        log_error "PostgreSQL is not running"
        return 1
    fi
    log_success "PostgreSQL is ready"
    
    if ! is_service_running $ADMIN_SERVER_PORT; then
        log_error "Admin Server is not running on port $ADMIN_SERVER_PORT"
        return 1
    fi
    log_success "Admin Server is ready"
    
    if ! is_service_running $ADMIN_UI_PORT; then
        log_error "Admin UI is not running on port $ADMIN_UI_PORT"
        return 1
    fi
    log_success "Admin UI is ready"
    
    log_success "All services are ready!"
    return 0
}

# Run tests
run_tests() {
    log_info "Running tests..."
    
    TESTS_PASSED=0
    TESTS_TOTAL=4
    
    # Test 1: Get Server Status
    echo "--- Test 1: Get Server Status ---"
    RESULT=$(curl -s "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/health/status")
    if echo "$RESULT" | grep -q "status"; then
        log_success "Server status endpoint working"
        TESTS_PASSED=$((TESTS_PASSED + 1))
        echo -e "${GREEN}✓ Test 1 PASSED${NC}"
    else
        log_error "Server status endpoint failed"
        echo -e "${RED}✗ Test 1 FAILED${NC}"
    fi
    echo ""
    
    # Test 2: Get Server Metrics
    echo "--- Test 2: Get Server Metrics ---"
    RESULT=$(curl -s "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/health/metrics")
    if echo "$RESULT" | grep -q "cpu\|memory"; then
        log_success "Server metrics endpoint working"
        TESTS_PASSED=$((TESTS_PASSED + 1))
        echo -e "${GREEN}✓ Test 2 PASSED${NC}"
    else
        log_error "Server metrics endpoint failed"
        echo -e "${RED}✗ Test 2 FAILED${NC}"
    fi
    echo ""
    
    # Test 3: Get Server Version
    echo "--- Test 3: Get Server Version ---"
    RESULT=$(curl -s "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/health/version")
    if echo "$RESULT" | grep -q "version"; then
        log_success "Server version endpoint working"
        TESTS_PASSED=$((TESTS_PASSED + 1))
        echo -e "${GREEN}✓ Test 3 PASSED${NC}"
    else
        log_error "Server version endpoint failed"
        echo -e "${RED}✗ Test 3 FAILED${NC}"
    fi
    echo ""
    
    # Test 4: Get Server Config
    echo "--- Test 4: Get Server Config ---"
    RESULT=$(curl -s "http://localhost:$ADMIN_SERVER_PORT/api/v1/admin/server/config")
    if echo "$RESULT" | grep -q "config\|server"; then
        log_success "Server config endpoint working"
        TESTS_PASSED=$((TESTS_PASSED + 1))
        echo -e "${GREEN}✓ Test 4 PASSED${NC}"
    else
        log_error "Server config endpoint failed"
        echo -e "${RED}✗ Test 4 FAILED${NC}"
    fi
    echo ""
    
    # Print summary
    echo "========================================"
    echo "  Test Summary"
    echo "========================================"
    echo "  Passed: $TESTS_PASSED / $TESTS_TOTAL"
    echo "========================================"
    
    if [ $TESTS_PASSED -eq $TESTS_TOTAL ]; then
        log_success "All Server Control tests passed!"
        return 0
    else
        log_error "Some tests failed"
        return 1
    fi
}

# Cleanup
cleanup() {
    log_info "Cleaning up..."
    
    if [ ! -z "$ADMIN_SERVER_PID" ]; then
        kill $ADMIN_SERVER_PID 2>/dev/null || true
    fi
    
    if [ ! -z "$ADMIN_UI_PID" ]; then
        kill $ADMIN_UI_PID 2>/dev/null || true
    fi
    
    pkill -f "palpo-admin-server" 2>/dev/null || true
    pkill -f "dx serve" 2>/dev/null || true
}

# Main
main() {
    local mode="${1:-full}"
    
    echo "========================================"
    echo "  Server Control E2E Tests"
    echo "========================================"
    echo ""
    
    case "$mode" in
        setup)
            start_postgres
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
            log_info "Cleaning test data..."
            # Add cleanup logic here if needed
            ;;
        restart)
            cleanup
            sleep 1
            start_postgres
            start_admin_server
            start_admin_ui
            check_services
            ;;
        full)
            trap cleanup EXIT
            start_postgres
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

main "$@"
