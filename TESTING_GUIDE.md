# Palpo Project Testing Guide

This document provides testing guidance for developers working on the Palpo project, covering test types, execution methods, and project-specific testing requirements.

## Table of Contents

1. [Test Types](#test-types)
2. [Running Tests](#running-tests)
3. [Writing Tests](#writing-tests)
4. [Testing Tools](#testing-tools)
5. [CI/CD Pipeline](#cicd-pipeline)
6. [Common Issues](#common-issues)
7. [Reference Resources](#reference-resources)

---

## Test Types

### 1. Unit Tests

**Location**: `tests.rs` modules within source files

**Examples**:
- `crates/core/src/metadata/tests.rs` - Metadata tests
- `crates/core/src/signatures/tests.rs` - Signature verification tests
- `crates/core/src/state/tests.rs` - State resolution tests
- `crates/core/src/state/event_auth/tests.rs` - Event authorization tests

**Characteristics**:
- Test individual functions or modules
- Fast execution (seconds)
- No external dependencies required

### 2. Complement Tests

**Description**: Official Matrix specification compliance test suite written in Go.

**Location**: `tests/complement/`

**Contents**:
- Client-Server API tests (CSAPI)
- Matrix Spec Change tests (MSC)
- Docker build configurations and startup scripts

**Purpose**: Verify that Palpo implementation complies with Matrix specifications.

**Details**: See [tests/README.md](tests/README.md) and [Complement repository](https://github.com/matrix-org/complement).

### 3. SyTest Tests

**Description**: Another Matrix specification test framework providing additional compatibility verification.

**Location**: `tests/sytest/`

**Contents**:
- `sytest-whitelist` - List of passed tests
- `sytest-blacklist` - List of failed tests
- `are-we-synapse-yet.py` - Test result analysis tool

**Purpose**: Supplemental verification of Matrix specification compatibility.

---

## Running Tests

### Basic Test Commands

```bash
# Run all tests
cargo test --all --all-features

# Run tests for a specific crate
cargo test -p palpo-core

# Run specific test
cargo test test_name

# Show test output (including println! etc.)
cargo test -- --nocapture

# Run tests without fast fail (show all failures)
cargo test --no-fail-fast
```

### Running Complement Tests

**Prerequisites**:
1. Install Docker
2. Clone Complement repository

```bash
# Clone Complement
git clone https://github.com/matrix-org/complement.git ../complement

# Run tests
bash ./tests/complement.sh ../complement test.log test.jsonl
```

**Test Process**:
1. Build base Docker image (`complement-palpo-base`)
2. Build test Docker image (`complement-palpo-test`)
3. Run Go test suite
4. Generate test logs (`test.log`) and result files (`test.jsonl`)

### Running SyTest Tests

**Prerequisites**:
1. Install Docker
2. Clone SyTest and configuration repositories

```bash
# Clone repositories
git clone https://github.com/matrix-org/sytest.git ../sytest
git clone https://github.com/palpo-im/sytest-palpo.git ../sytest-palpo

# Run tests
bash ./tests/run-sytest.sh
```

**Test Output**:
- Test logs stored in `tests/sytestout/logs/`
- Use `tests/sytest/are-we-synapse-yet.py` to analyze results

### Pre-commit Full Check

Run complete quality checks before committing code:

```bash
# 1. Code formatting check
cargo fmt --all -- --check

# 2. Code formatting (auto-fix)
cargo +nightly fmt --all

# 3. Clippy static analysis
cargo clippy --all --all-features -- -D warnings

# 4. Build check
cargo check --all --bins --examples --tests

# 5. Release build check
cargo check --all --bins --examples --tests --release

# 6. Run all tests
cargo test --all --all-features --no-fail-fast

# 7. Spell check
typos
```

---

## Writing Tests

### Test Requirements

According to [CONTRIBUTING.md](CONTRIBUTING.md) requirements:

1. **Add tests for new features**
   - Add unit tests for new functions and modules
   - Add integration tests for API endpoints

2. **Test coverage**
   - Test success paths
   - Test error paths

3. **Use descriptive test names**
   - Test names should clearly indicate test content

### Test Examples

**Basic unit test structure**:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_function_name() {
        // Test success scenario
        let result = function_to_test(valid_input);
        assert!(result.is_ok());

        // Test error scenario
        let result = function_to_test(invalid_input);
        assert!(result.is_err());
    }
}
```

---

## Testing Tools

### Test Utility Module

The project provides test utility functions in `crates/core/src/test_utils.rs`:

**Available Functions**:
- `alice()`, `bob()` - Create test users
- `room_id()` - Create test room ID
- `event_id()` - Create test event ID
- `to_pdu_event()` - Create PDU event
- `TestStore` - Mock event storage
- `do_check()` - State resolution verification
- `INITIAL_EVENTS()` - Initialize test events

**Usage Example**:

```rust
use crate::test_utils::{alice, bob, room_id};

#[test]
fn test_room_membership() {
    let user = alice();      // Create test user Alice
    let room = room_id();    // Create test room ID
    let event = to_pdu_event(/* ... */);  // Create PDU event

    // Use test utility functions
    let result = process_event(&event);
    assert!(result.is_ok());
}
```

### Snapshot Tests

The project uses `insta` for snapshot testing:

```rust
use insta::assert_snapshot;

#[test]
fn test_signature_output() {
    let signature = generate_signature();
    insta::assert_snapshot!(signature);
}
```

**Snapshot Testing Workflow**:

```bash
# Run tests
cargo test

# If snapshots don't match, review differences
cargo insta review

# Accept new snapshots (if changes are expected)
cargo insta accept

# Reject changes
cargo insta reject
```

---

## CI/CD Pipeline

### GitHub Actions Workflow

The project uses `.github/workflows/quality-control.yml` for automated testing.

**Trigger Conditions**:
- Pull Request (opened, synchronized, reopened)
- Push to `main` branch
- Triggered only when the following files change:
  - `**.rs` - Rust source files
  - `**/Cargo.toml` - Dependency configuration
  - `.github/workflows/**` - Workflow configuration

**Test Tasks**:

1. **Spell Check** (typos)
2. **Build Check**
   - Debug version: `cargo check --all --bins --examples --tests`
   - Release version: `cargo check --all --bins --examples --tests --release`
3. **Run Tests**
   - Command: `cargo test --all --all-features --no-fail-fast -- --nocapture`
   - Timeout: 40 minutes

**Test Environment**:
- Operating System: Ubuntu Latest
- Rust Version: Stable
- Architecture: x86_64-unknown-linux-gnu

---

## Common Issues

### 1. Test Failure: PostgreSQL Not Found

**Issue**: Tests require PostgreSQL but it's not installed or running.

**Solution**:

```bash
# Ubuntu/Debian
sudo apt install postgresql postgresql-contrib
sudo systemctl start postgresql

# macOS
brew install postgresql
brew services start postgresql
```

### 2. Complement Test Failure: Docker Image Issues

**Issue**: Docker image build fails or doesn't exist.

**Solution**:

```bash
# Clean old images
docker rmi complement-palpo-base complement-palpo-test

# Re-run test script (will automatically rebuild)
bash ./tests/complement.sh ../complement test.log test.jsonl
```

### 3. Test Timeout

**Issue**: Some tests take too long to run.

**Solution**:

```bash
# Reduce concurrent threads
cargo test -- --test-threads=1 --nocapture

# Or run specific test
cargo test specific_test_name
```

### 4. Snapshot Test Mismatch

**Issue**: `insta` snapshot tests fail.

**Solution**:

```bash
# Review snapshot differences
cargo insta review

# If changes are expected, accept new snapshots
cargo insta accept
```

### 5. Clippy Warnings

**Issue**: Code doesn't comply with Clippy standards.

**Solution**:

```bash
# View detailed warnings
cargo clippy --all --all-features

# Auto-fix some issues
cargo clippy --fix --all --all-features
```

**Note**: The project configures allowed warnings in `Cargo.toml` (such as `result_large_err`, `type_complexity`, etc.). Only unconfigured warnings will cause build failures.

### 6. Spell Check Failure

**Issue**: `typos` tool detects spelling errors.

**Solution**:

```bash
# View spelling errors
typos

# If it's a proper noun, add to typos.toml configuration
# See typos.toml file in project root
```

---

## Reference Resources

- [CONTRIBUTING.md](CONTRIBUTING.md) - Project contribution guide
- [tests/README.md](tests/README.md) - Complement test documentation
- [Rust Testing Documentation](https://doc.rust-lang.org/book/ch11-00-testing.html)
- [Complement Test Framework](https://github.com/matrix-org/complement)
- [SyTest Test Framework](https://github.com/matrix-org/sytest)
- [Matrix Specification](https://spec.matrix.org/)

---

**Last Updated**: 2026
**Maintainer**: Palpo Development Team