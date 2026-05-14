# claudex — task runner
# Run `just` to list available recipes.

set shell := ["bash", "-cu"]

default:
    @just --list

# Build a debug binary.
build:
    cargo build

# Build an optimized release binary.
release:
    cargo build --release

# Run the CLI; forward extra args, e.g. `just run -- list claude`.
run *args:
    cargo run -- {{args}}

# Run tests with nextest if available, falling back to cargo test.
test *args:
    @if command -v cargo-nextest >/dev/null 2>&1; then \
        cargo nextest run {{args}}; \
    else \
        cargo test {{args}}; \
    fi

# Watch sources and re-run tests on change.
watch:
    cargo watch -x 'check --all-targets' -x test

# Format the codebase.
fmt:
    cargo fmt --all

# Check formatting without modifying files.
fmt-check:
    cargo fmt --all -- --check

# Lint with clippy; warnings are errors.
lint:
    cargo clippy --all-targets --all-features -- -D warnings

# Full CI gate: format check, lint, build, test.
ci: fmt-check lint build test

# Install the release binary into ~/.cargo/bin (or $CARGO_INSTALL_ROOT).
install:
    cargo install --path . --locked --force

# Uninstall the binary.
uninstall:
    cargo uninstall claudex

# Remove build artifacts.
clean:
    cargo clean
