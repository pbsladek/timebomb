SHELL         := bash
.SHELLFLAGS   := -euo pipefail -c
.ONESHELL:
.DEFAULT_GOAL := help
.DELETE_ON_ERROR:
MAKEFLAGS     += --warn-undefined-variables --no-builtin-rules

BINARY         := timebomb
RELEASE_BINARY := target/release/$(BINARY)
CARGO          := cargo

# Smoke-test working directory.  Override to avoid collisions on shared runners:
#   make smoke SMOKE_DIR=/my/tmp/smoke
SMOKE_DIR ?= /tmp/timebomb-smoke

export CARGO_TERM_COLOR := always
export RUST_BACKTRACE   := 1

# ── Smoke fixture helper ──────────────────────────────────────────────────────
#
# Creates a fixture directory and writes a single source file into it.
# Defined as a single-line macro so it works as a recipe expansion.
# Usage: $(call write-fixture,<subdir>,<source-content>,<filename>)
write-fixture = mkdir -p "$(SMOKE_DIR)/$(1)" && \
                printf '%s\n' '$(2)' > "$(SMOKE_DIR)/$(1)/$(3)"

# ── Phony targets ─────────────────────────────────────────────────────────────

.PHONY: help \
        build build-release build-dist \
        test-unit test-integration test test-nocapture bench bench-no-run \
        fmt fmt-check clippy lint \
        smoke smoke-empty smoke-list smoke-expired smoke-json smoke-github smoke-clean \
        check ci self-check self-list run \
        install install-dist uninstall \
        clean clean-smoke clean-bench

# ── Help ──────────────────────────────────────────────────────────────────────

##@ General

help:  ## Show this help
	@awk 'BEGIN {FS = ":.*##"; printf "\nUsage:\n  make \033[36m<target>\033[0m\n"} \
	      /^[a-zA-Z_-]+:.*?##/ { printf "  \033[36m%-22s\033[0m %s\n", $$1, $$2 } \
	      /^##@/ { printf "\n\033[1m%s\033[0m\n", substr($$0, 5) }' \
	      $(MAKEFILE_LIST)

##@ Build

build:  ## Compile (dev profile)
	$(CARGO) build

build-release:  ## Compile (release profile)
	$(CARGO) build --release

build-dist:  ## Compile (dist profile — thin-LTO, matches release pipeline)
	$(CARGO) build --profile dist

##@ Test

test-unit:  ## Run inline unit tests — mirrors CI unit-tests job
	$(CARGO) test --lib --bins --verbose

test-integration:  ## Run integration tests (tests/) — mirrors CI integration-tests job
	$(CARGO) test --tests --verbose

test: test-unit test-integration  ## Run all tests (unit + integration)

test-nocapture:  ## Run all tests showing eprintln! output
	$(CARGO) test -- --nocapture

bench:  ## Run criterion benchmarks and print a formatted summary table. Pass TIME=<secs> to change measurement time per bench (default: 5)
	@./benches/bench.sh $(if $(TIME),--time $(TIME),)

bench-no-run:  ## Reformat last saved benchmark results without re-running
	@./benches/bench.sh --no-run

##@ Lint

fmt:  ## Format source code in place
	$(CARGO) fmt

fmt-check:  ## Check formatting without modifying files — mirrors CI fmt job
	$(CARGO) fmt --all -- --check

clippy:  ## Lint with clippy -D warnings — mirrors CI clippy job
	$(CARGO) clippy --all-targets --all-features -- -D warnings

lint: fmt-check clippy  ## Run all linters (fmt-check + clippy)

##@ Smoke Tests

smoke-empty: build-release  ## Smoke: sweep exits 0 on an empty directory
	@mkdir -p "$(SMOKE_DIR)/empty"
	printf '  %-40s' 'empty dir (exits 0) ...'
	$(RELEASE_BINARY) sweep "$(SMOKE_DIR)/empty" > /dev/null 2>&1
	printf '\033[32m✓ pass\033[0m\n'

smoke-list: build-release  ## Smoke: manifest exits 0 even with detonated fuses
	@$(call write-fixture,list,// TODO[2020-01-01]: expired annotation,test.rs)
	printf '  %-40s' 'manifest with detonated (exits 0) ...'
	$(RELEASE_BINARY) manifest "$(SMOKE_DIR)/list" > /dev/null 2>&1
	printf '\033[32m✓ pass\033[0m\n'

smoke-expired: build-release  ## Smoke: sweep exits 1 when a detonated fuse is found
	@$(call write-fixture,expired,// TODO[2020-01-01]: this is expired,main.rs)
	printf '  %-40s' 'detonated fuse (exits 1) ...'
	$(RELEASE_BINARY) sweep "$(SMOKE_DIR)/expired" > /dev/null 2>&1 && { printf '\033[31m✗ FAIL\033[0m  (expected exit 1, got 0)\n' >&2; exit 1; } || true
	printf '\033[32m✓ pass\033[0m\n'

smoke-json: build-release  ## Smoke: --format json produces valid JSON
	@$(call write-fixture,json,// FIXME[2020-01-01]: old,lib.rs)
	printf '  %-40s' '--format json (valid JSON output) ...'
	{ $(RELEASE_BINARY) sweep "$(SMOKE_DIR)/json" --format json || true; } \
		| python3 -m json.tool > /dev/null
	printf '\033[32m✓ pass\033[0m\n'

smoke-github: build-release  ## Smoke: --format github emits ::error workflow commands
	@$(call write-fixture,github,// TODO[2020-01-01]: expired annotation,main.rs)
	printf '  %-40s' '--format github (::error present) ...'
	($(RELEASE_BINARY) sweep "$(SMOKE_DIR)/github" --format github 2>&1 || true) | grep -q "::error" || { printf '\033[31m✗ FAIL\033[0m  (::error not found)\n' >&2; exit 1; }
	printf '\033[32m✓ pass\033[0m\n'

smoke: build-release smoke-empty smoke-list smoke-expired smoke-json smoke-github  ## Run all smoke tests
	@printf '\n\033[1;32m✓ All smoke tests passed\033[0m\n'

smoke-clean:  ## Remove smoke test temp files
	@rm -rf "$(SMOKE_DIR)"

##@ Dev Workflow

check: fmt-check clippy test smoke  ## Run the full CI pipeline locally (lint → test → smoke)
	@printf '\n\033[1;32m✓ All checks passed\033[0m\n'

ci: check  ## Alias for check

self-check: build-release  ## Sweep src/ with GitHub Actions format (informational, always exits 0)
	$(RELEASE_BINARY) sweep ./src --format github || true

self-list: build-release  ## Manifest all fuses in src/ sorted by date
	$(RELEASE_BINARY) manifest ./src || true

run:  ## Run the dev binary: make run ARGS="check ./src"
	$(CARGO) run -- $(ARGS)

##@ Install

install:  ## Install to ~/.cargo/bin (release profile)
	$(CARGO) install --path .

install-dist:  ## Install to ~/.cargo/bin (dist profile — thin-LTO)
	$(CARGO) install --path . --profile dist

uninstall:  ## Uninstall from ~/.cargo/bin
	$(CARGO) uninstall $(BINARY)

##@ Clean

clean:  ## Remove all build artifacts (target/)
	$(CARGO) clean

clean-smoke: smoke-clean  ## Alias for smoke-clean

clean-bench:  ## Remove criterion benchmark reports (target/criterion/)
	rm -rf target/criterion
