# solex task runner

default:
    @just --list

# Format all crates
fmt:
    cargo fmt --all -- --check
    cargo fmt --manifest-path tests/native/Cargo.toml -- --check
    cargo fmt --manifest-path tests/pinocchio/Cargo.toml -- --check
    cargo fmt --manifest-path examples/counter/Cargo.toml -- --check

fmt-fix:
    cargo fmt --all
    cargo fmt --manifest-path tests/native/Cargo.toml
    cargo fmt --manifest-path tests/pinocchio/Cargo.toml
    cargo fmt --manifest-path examples/counter/Cargo.toml

# Clippy with warnings as errors
lint:
    cargo clippy --workspace --all-targets -- -D warnings
    cargo clippy --manifest-path tests/native/Cargo.toml --all-targets -- -D warnings

# Host tests: workspace crates (Anchor integration included) + native tests per channel
test: test-workspace test-native test-pinocchio

test-workspace:
    cargo test --workspace

# Framework-free macro tests, compiled once per fixture channel
# `--features hooks` mirrors what `solex test --channel <c>` would pass.
test-native:
    cargo test --manifest-path tests/native/Cargo.toml
    SOLEX_CHANNEL=nightly cargo test --manifest-path tests/native/Cargo.toml --features hooks
    SOLEX_CHANNEL=canary cargo test --manifest-path tests/native/Cargo.toml --features hooks

# no_std Pinocchio compile check for the `pinocchio` feature, per channel
test-pinocchio:
    SOLEX_CHANNEL=mainnet cargo check --manifest-path tests/pinocchio/Cargo.toml
    SOLEX_CHANNEL=devnet cargo check --manifest-path tests/pinocchio/Cargo.toml

# Build the CLI
build:
    cargo build --release -p solex

# Install the CLI locally
install:
    cargo install --path crates/cli --force

# Type-check the Anchor example on the host for every channel
example-check:
    cd examples/counter && SOLEX_CHANNEL=stable cargo check
    cd examples/counter && SOLEX_CHANNEL=nightly cargo check

# Build the example program for every channel (requires cargo build-sbf)
example-build:
    cargo build -p solex
    cd examples/counter && ../../target/debug/solex build --all

# Full CI gate
ci: fmt lint test example-check
