#!/bin/bash

# Master E2E Test Runner
# Coordinates all E2E tests for the Palpo Admin UI
# Runs tests in the correct order: health check → server control → user management

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

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

log_section() {
    echo ""
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${CYAN}  $1${NC}"
    echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
}

# Parse arguments
TESTS_TO_RUN="all"
for arg in "$@"; do
    case "$arg" in
        --health)      TESTS_TO_RUN="health" ;;
        --control)     TESTS_TO_RUN="control" ;;
        --users)       TESTS_TO_RUN="users" ;;
        --setup)       TESTS_TO_RUN="setup" ;;
        --clean)       TESTS_TO_RUN="clean" ;;
        --help)        show_help; exit 0 ;;
    esac
done

show_help() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  --health      Run only admin server health tests"
    echo "  --control     Run only server control tests"
    echo "  --users       Run only user management tests"
    echo "  --setup       Setup services (no tests)"
    echo "  --clean       Clean test data"
    echo "  --help        Show this help message"
    echo ""
    echo "Default (no options): Run all tests"
}

# Run admin server health tests
run_health_tests() {
    log_section "Admin Server Health Tests"
    
    if [ ! -f "$SCRIPT_DIR/e2e_admin_server_health.sh" ]; then
        log_error "Health test script not found: $SCRIPT_DIR/e2e_admin_server_health.sh"
        return 1
    fi
    
    bash "$SCRIPT_DIR/e2e_admin_server_health.sh" --test
    local result=$?
    
    if [ $result -eq 0 ]; then
        log_success "Admin server health tests passed"
        return 0
    else
        log_error "Admin server health tests failed"
        return 1
    fi
}

# Run server control tests
run_control_tests() {
    log_section "Palpo Server Control Tests"
    
    if [ ! -f "$SCRIPT_DIR/e2e_server_control.sh" ]; then
        log_error "Server control test script not found: $SCRIPT_DIR/e2e_server_control.sh"
        return 1
    fi
    
    bash "$SCRIPT_DIR/e2e_server_control.sh" --test
    local result=$?
    
    if [ $result -eq 0 ]; then
        log_success "Server control tests passed"
        return 0
    else
        log_error "Server control tests failed"
        return 1
    fi
}

# Run Palpo health tests
run_palpo_health_tests() {
    log_section "Palpo Server Health Tests"
    
    if [ ! -f "$SCRIPT_DIR/e2e_palpo_health.sh" ]; then
        log_error "Palpo health test script not found: $SCRIPT_DIR/e2e_palpo_health.sh"
        return 1
    fi
    
    bash "$SCRIPT_DIR/e2e_palpo_health.sh" --test
    local result=$?
    
    if [ $result -eq 0 ]; then
        log_success "Palpo health tests passed"
        return 0
    else
        log_error "Palpo health tests failed"
        return 1
    fi
}

# Run user management tests
run_user_tests() {
    log_section "User Management Tests"
    
    if [ ! -f "$SCRIPT_DIR/e2e_user_management.sh" ]; then
        log_error "User management test script not found: $SCRIPT_DIR/e2e_user_management.sh"
        return 1
    fi
    
    bash "$SCRIPT_DIR/e2e_user_management.sh" --test
    local result=$?
    
    if [ $result -eq 0 ]; then
        log_success "User management tests passed"
        return 0
    else
        log_error "User management tests failed"
        return 1
    fi
}

# Setup services
setup_services() {
    log_section "Setting Up Services"
    
    if [ ! -f "$SCRIPT_DIR/e2e_server_control.sh" ]; then
        log_error "Server control script not found: $SCRIPT_DIR/e2e_server_control.sh"
        return 1
    fi
    
    bash "$SCRIPT_DIR/e2e_server_control.sh" --setup
    local result=$?
    
    if [ $result -eq 0 ]; then
        log_success "Services setup completed"
        return 0
    else
        log_error "Services setup failed"
        return 1
    fi
}

# Clean test data
clean_test_data() {
    log_section "Cleaning Test Data"
    
    if [ ! -f "$SCRIPT_DIR/e2e_server_control.sh" ]; then
        log_error "Server control script not found: $SCRIPT_DIR/e2e_server_control.sh"
        return 1
    fi
    
    bash "$SCRIPT_DIR/e2e_server_control.sh" --clean
    local result=$?
    
    if [ $result -eq 0 ]; then
        log_success "Test data cleanup completed"
        return 0
    else
        log_error "Test data cleanup failed"
        return 1
    fi
}

# Main
main() {
    echo "========================================"
    echo "  Palpo Admin UI - E2E Test Suite"
    echo "========================================"
    echo ""
    
    TESTS_PASSED=0
    TESTS_FAILED=0
    
    case "$TESTS_TO_RUN" in
        setup)
            setup_services
            exit $?
            ;;
        clean)
            clean_test_data
            exit $?
            ;;
        health)
            run_health_tests
            exit $?
            ;;
        control)
            run_control_tests
            exit $?
            ;;
        users)
            run_user_tests
            exit $?
            ;;
        all)
            # Run all tests in order
            
            # 1. Setup services
            setup_services || {
                log_error "Failed to setup services"
                exit 1
            }
            
            # 2. Run admin server health tests
            run_health_tests
            if [ $? -eq 0 ]; then
                TESTS_PASSED=$((TESTS_PASSED + 1))
            else
                TESTS_FAILED=$((TESTS_FAILED + 1))
            fi
            
            # 3. Run server control tests
            run_control_tests
            if [ $? -eq 0 ]; then
                TESTS_PASSED=$((TESTS_PASSED + 1))
            else
                TESTS_FAILED=$((TESTS_FAILED + 1))
            fi
            
            # 4. Run Palpo health tests
            run_palpo_health_tests
            if [ $? -eq 0 ]; then
                TESTS_PASSED=$((TESTS_PASSED + 1))
            else
                TESTS_FAILED=$((TESTS_FAILED + 1))
            fi
            
            # 5. Run user management tests
            run_user_tests
            if [ $? -eq 0 ]; then
                TESTS_PASSED=$((TESTS_PASSED + 1))
            else
                TESTS_FAILED=$((TESTS_FAILED + 1))
            fi
            
            # 6. Cleanup
            clean_test_data
            
            # Print summary
            log_section "Test Summary"
            echo "  Tests Passed: $TESTS_PASSED"
            echo "  Tests Failed: $TESTS_FAILED"
            echo "========================================"
            
            if [ $TESTS_FAILED -eq 0 ]; then
                log_success "All tests passed!"
                exit 0
            else
                log_error "Some tests failed"
                exit 1
            fi
            ;;
        *)
            show_help
            exit 1
            ;;
    esac
}

main
