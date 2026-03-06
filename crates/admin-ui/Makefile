# Makefile for Palpo Admin UI

.PHONY: help dev build test check clean install

# Default target
.DEFAULT_GOAL := help

# Colors for output
BLUE := \033[0;34m
GREEN := \033[0;32m
YELLOW := \033[0;33m
NC := \033[0m # No Color

help: ## Show this help message
	@echo "$(BLUE)Palpo Admin UI - Available Commands$(NC)"
	@echo ""
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "  $(GREEN)%-15s$(NC) %s\n", $$1, $$2}'
	@echo ""

install: ## Install dependencies and tools
	@echo "$(BLUE)Installing dependencies...$(NC)"
	@rustup target add wasm32-unknown-unknown
	@if ! command -v dx &> /dev/null; then \
		echo "$(YELLOW)Installing Dioxus CLI...$(NC)"; \
		cargo install dioxus-cli; \
	fi
	@echo "$(GREEN)✅ Installation complete$(NC)"

dev: ## Start development server with hot reload
	@./scripts/dev.sh

dev-no-open: ## Start development server without opening browser
	@./scripts/dev.sh --no-open

build: ## Build release version
	@./scripts/build.sh

build-debug: ## Build debug version
	@./scripts/build.sh --debug

test: ## Run all tests
	@./scripts/test.sh

test-unit: ## Run unit tests only
	@./scripts/test.sh --unit

test-integration: ## Run integration tests only
	@./scripts/test.sh --integration

test-watch: ## Run tests in watch mode
	@./scripts/test.sh --watch

check: ## Run code quality checks (check + clippy + fmt)
	@./scripts/check.sh

check-fix: ## Run code quality checks and auto-fix issues
	@./scripts/check.sh --fix

fmt: ## Format code
	@echo "$(BLUE)Formatting code...$(NC)"
	@cargo fmt --all
	@echo "$(GREEN)✅ Code formatted$(NC)"

clippy: ## Run clippy linter
	@echo "$(BLUE)Running clippy...$(NC)"
	@cargo clippy --all-targets -- -D warnings
	@echo "$(GREEN)✅ Clippy check passed$(NC)"

clean: ## Clean build artifacts
	@./scripts/clean.sh

verify: check test ## Run all checks and tests

# Quick aliases
d: dev ## Alias for 'dev'
b: build ## Alias for 'build'
t: test ## Alias for 'test'
c: check ## Alias for 'check'
