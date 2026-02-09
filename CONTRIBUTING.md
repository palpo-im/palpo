# Contributing to Palpo

Thank you for your interest in contributing to Palpo! We welcome contributions from everyone, whether you're fixing bugs, adding features, improving documentation, or writing tests.

## Table of Contents

- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [Making Contributions](#making-contributions)
- [Code Style](#code-style)
- [Testing](#testing)
- [Pull Request Process](#pull-request-process)
- [Reporting Issues](#reporting-issues)

## Getting Started

Before contributing, please:

1. Read this guide in full
2. Check [existing issues](https://github.com/palpo-im/palpo/issues) to avoid duplicates
3. For significant changes, open an issue first to discuss your proposal

## Development Setup

### Prerequisites

- **Rust 1.92+** - Install via [rustup](https://rustup.rs/)
- **PostgreSQL 18+** - Required for the database backend
- **System dependencies** (Linux):
  ```bash
  sudo apt install -y libclang-dev libpq-dev cmake
  ```

### Building from Source

```bash
# Clone the repository
git clone https://github.com/palpo-im/palpo.git
cd palpo

# Build the project
cargo build

# Run all checks (recommended before committing)
cargo check --all --bins --examples --tests
```

### Admin Console

Palpo includes an interactive admin console for managing the server:

```bash
# Console-only mode (no HTTP server)
./palpo -c palpo.toml -s false --console

# Console + server mode
./palpo -c palpo.toml --console
```

**Console commands:**

| Command | Description |
|---------|-------------|
| `help` | Show available admin commands |
| `exit` / `quit` | Exit the console (and server if running) |
| `clear` | Clear the terminal screen |
| `user <subcommand>` | Manage local users |
| `room <subcommand>` | Manage rooms |
| `federation <subcommand>` | Manage federation |
| `server <subcommand>` | Manage server settings |
| `media <subcommand>` | Manage media |
| `appservice <subcommand>` | Manage appservices |

**Keyboard shortcuts:**

| Shortcut | Description |
|----------|-------------|
| `Ctrl+D` | Exit the console (same as `exit`) |
| `Ctrl+C` | Interrupt the current command |
| `Up/Down` | Navigate command history |

**Graceful shutdown:**

When running in console+server mode, exiting the console (via `exit`, `quit`, or `Ctrl+D`) triggers a graceful server shutdown. The server will stop accepting new connections and exit cleanly.

When running as a daemon (without `--console`), send `SIGTERM` or `SIGQUIT` for graceful shutdown:

```bash
kill -TERM <palpo-pid>
```

### Project Structure

Palpo is organized as a Cargo workspace with the following crates:

| Crate | Description |
|-------|-------------|
| `palpo` | Main server application |
| `palpo-core` | Core library with Matrix protocol types |
| `palpo-core-macros` | Procedural macros for the core library |
| `palpo-data` | Data layer with Diesel ORM integration |
| `palpo-identifiers-validation` | User/room identifier validation |
| `palpo-server-macros` | Server-specific procedural macros |

## Making Contributions

### Types of Contributions

We appreciate all kinds of contributions:

- **Bug fixes** - Fix issues reported in the tracker
- **Features** - Implement new functionality (discuss first in an issue)
- **Documentation** - Improve docs, add examples, fix typos
- **Tests** - Add test coverage, improve existing tests
- **Performance** - Optimize code, reduce memory usage

### Before You Start

1. **Create a branch** from `main` for your work
2. **Keep changes focused** - One logical change per PR
3. **Write tests** for new functionality
4. **Update documentation** if needed

## Code Style

### Rust Formatting

We use `rustfmt` with the Rust 2024 edition style. Format your code before committing:

```bash
cargo +nightly fmt --all
```

### Linting

We use `clippy` for linting. Your code should pass without warnings:

```bash
cargo clippy --all --all-features -- -D warnings
```

### Spell Checking

We use [typos](https://github.com/crate-ci/typos) to catch spelling mistakes:

```bash
# Install typos
cargo install typos-cli

# Run spell check
typos
```

### Style Guidelines

- Use meaningful variable and function names
- Keep functions focused and reasonably sized
- Add comments for complex logic (not obvious code)
- Follow existing patterns in the codebase
- Prefer explicit error handling over `.unwrap()` in production code

## Testing

### Running Tests

```bash
# Run all tests
cargo test --all --all-features

# Run tests for a specific crate
cargo test -p palpo-core

# Run a specific test
cargo test test_name
```

### Complement Testing

We use [Complement](https://github.com/matrix-org/complement) for Matrix specification compliance testing. See the [tests/README.md](tests/README.md) for details.

### Writing Tests

- Add unit tests for new functions and modules
- Add integration tests for API endpoints
- Test both success and error cases
- Use descriptive test names that explain what is being tested

## Pull Request Process

### Before Submitting

Ensure your changes pass all CI checks locally:

```bash
# Format check
cargo fmt --all -- --check

# Linting
cargo clippy --all --all-features -- -D warnings

# Build check
cargo check --all --bins --examples --tests

# Tests
cargo test --all --all-features --no-fail-fast

# Spell check
typos
```

### Submitting a PR

1. **Create a descriptive title** - Summarize the change clearly
2. **Write a helpful description** - Explain what and why
3. **Reference related issues** - Use `Fixes #123` or `Relates to #123`
4. **Keep it reviewable** - Smaller PRs are easier to review and merge

### PR Title Format

Use a clear, descriptive title:

- `fix: resolve login failure on invalid tokens`
- `feat: add support for room aliases`
- `docs: update installation guide`
- `refactor: simplify event handling logic`
- `test: add tests for federation endpoints`

### Review Process

- A maintainer will review your PR
- Address feedback and update your PR as needed
- Once approved, a maintainer will merge your PR

## Reporting Issues

### Bug Reports

Use the [bug report template](.github/workflows/ISSUE_TEMPLATE/BUG_REPORT.md) and include:

- Palpo version or git SHA
- Whether you're running in Docker
- Client used (if applicable)
- Clear description of the problem
- Steps to reproduce

### Feature Requests

Use the [feature request template](.github/workflows/ISSUE_TEMPLATE/FEATURE_REQUEST.md) and:

- Describe the feature clearly
- Explain the use case
- Note: Don't request missing Matrix spec features as issues

## Questions?

- Check existing [issues](https://github.com/palpo-im/palpo/issues) and discussions
- Open a new issue for questions about contributing

---

Thank you for contributing to Palpo!
