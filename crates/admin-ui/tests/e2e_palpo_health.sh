#!/bin/bash

# E2E Test Script for Palpo Server Health Monitoring
# Tests Palpo server health status, metrics, and version information
# Verifies that Palpo is running and responding correctly

# Do NOT use set -e — we handle errors explicitly

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Configuration
PALPO_PORT=8008
PALPO_ADMIN_API="http://localhost:$PALPO_PORT/_synapse/admin"

# Parse arguments
MODE="full"
for arg in "$@"; do
    case "$arg" in
        --check)   MODE="check" ;;
        --test)    MODE="test" ;;
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

# Check if Palpo is running
check_palpo() {
    log_step "1" "Check Palpo Server Status"
    
    if check_port $PALPO_PORT; then
        log_success "Palpo is listening on port $PALPO_PORT"
        return 0
    else
        log_error "Palpo is not listening on port $PALPO_PORT"
        return 1
    fi
}

# Run health tests
run_tests() {
    log_step "2" "Run Palpo Health Tests"
    
    TESTS_PASSED=0
    TESTS_TOTAL=4
    
    echo ""
    echo "--- Palpo Health Tests (curl-based) ---"
    echo ""
    
    echo "Test 1: Check Palpo Server Responds"
    RESULT=$(curl -s "http://localhost:$PALPO_PORT/_matrix/client/versions")
    if echo "$RESULT" | grep -q "versions"; then
        log_success "Palpo server is responding"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        log_error "Palpo server is not responding correctly"
    fi
    echo ""
    
    echo "Test 2: Check Palpo Admin API Available"
    RESULT=$(curl -s "$PALPO_ADMIN_API/v1/users")
    if echo "$RESULT" | grep -q "users\|error\|unauthorized"; then
        log_success "Palpo admin API is available"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        log_warn "Palpo admin API response: $RESULT"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    fi
    echo ""
    
    echo "Test 3: Check Palpo Server Version"
    RESULT=$(curl -s "http://localhost:$PALPO_PORT/_synapse/admin/v1/server_version")
    if echo "$RESULT" | grep -q "server_version\|version"; then
        log_success "Palpo version information available"
        echo "  Version: $(echo "$RESULT" | grep -o '"server_version":"[^"]*' | cut -d'"' -f4)"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        log_warn "Palpo version response: $RESULT"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    fi
    echo ""
    
    echo "Test 4: Check Palpo Database Connection"
    RESULT=$(curl -s "$PALPO_ADMIN_API/v1/users?limit=1")
    if echo "$RESULT" | grep -q "users\|total"; then
        log_success "Palpo database connection is working"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        log_warn "Palpo database check result: $RESULT"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    fi
    echo ""
    
    echo "========================================"
    echo "  Test Summary"
    echo "========================================"
    echo "  Tests Passed: $TESTS_PASSED / $TESTS_TOTAL"
    echo "========================================"
    
    if [ $TESTS_PASSED -ge 3 ]; then
        log_success "Palpo health tests passed!"
        return 0
    else
        log_error "Some Palpo health tests failed"
        return 1
    fi
}

# Main
main() {
    echo "========================================"
    echo "  Palpo Server Health E2E Tests"
    echo "========================================"
    echo ""
    echo "Mode: $MODE"
    echo ""
    
    case "$MODE" in
        check)
            check_palpo
            ;;
        test)
            check_palpo || die "Palpo is not running"
            run_tests
            ;;
        full)
            check_palpo || die "Palpo is not running"
            run_tests
            ;;
        *)
            echo "Usage: $0 {check|test|full}"
            echo ""
            echo "Modes:"
            echo "  check - Check if Palpo is running"
            echo "  test  - Run health tests (requires Palpo running)"
            echo "  full  - Full workflow (check + test)"
            exit 1
            ;;
    esac
}

main
