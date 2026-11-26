# ==============================================================================
#  belt — justfile
# ==============================================================================

# Default recipe: list available recipes
default:
    @just --list

# ------------------------------------------------------------------------------
# Development & Quality
# ------------------------------------------------------------------------------

# Check syntax and types across all workspace targets
check:
    cargo check --workspace --all-targets

# Run tests across all workspace members
test:
    cargo test --workspace

# Run clippy linter across all workspace targets with warnings as errors
lint:
    cargo clippy --all-targets -- -D warnings

# Format all code files
fmt:
    cargo fmt

# Check formatting without modifying files
fmt-check:
    cargo fmt --check

# Full CI / QA gate, executed through fanout itself
qa:
    cargo run --quiet -p fanout -- check

# ------------------------------------------------------------------------------
# Build
# ------------------------------------------------------------------------------

# Build all crates in debug mode
build:
    cargo build --workspace

# Build all crates in release mode
build-release:
    cargo build --workspace --release

# ------------------------------------------------------------------------------
# Installation
# ------------------------------------------------------------------------------

bin_dir := env("BELT_BIN_DIR", env("HOME") / ".local/bin")

# Install a single tool into ~/.local/bin (override with BELT_BIN_DIR)
install tool:
    #!/usr/bin/env sh
    set -eu
    pkg={{tool}}
    [ "$pkg" = belt-pr ] && pkg=pr
    bin="$pkg"
    [ "$bin" = pr ] && bin=belt-pr
    cargo build --release -p "$pkg"
    mkdir -p "{{bin_dir}}"
    install -m 755 "${CARGO_TARGET_DIR:-target}/release/$bin" "{{bin_dir}}/$bin"
    printf '✓ %s/%s\n' "{{bin_dir}}" "$bin"
    just _shadowed "$bin"

# Install all CLI tools into ~/.local/bin (override with BELT_BIN_DIR)
install-all:
    #!/usr/bin/env sh
    set -eu
    cargo build --release --workspace
    mkdir -p "{{bin_dir}}"
    for bin in portkill jwt fanout webdriver belt-pr belt; do
        install -m 755 "${CARGO_TARGET_DIR:-target}/release/$bin" "{{bin_dir}}/$bin"
        printf '✓ %s/%s\n' "{{bin_dir}}" "$bin"
        just _shadowed "$bin"
    done

# Warn when an older copy earlier on PATH still answers to the name
_shadowed bin:
    @want="{{bin_dir}}/{{bin}}"; found=$(command -v "{{bin}}" || true); \
    if [ -n "$found" ] && [ "$found" != "$want" ]; then \
        echo "note: \`{{bin}}\` runs $found, which comes before $want on PATH; remove it or reorder PATH"; \
    fi

# ------------------------------------------------------------------------------
# Housekeeping
# ------------------------------------------------------------------------------

# Clean build artifacts
clean:
    cargo clean

# ------------------------------------------------------------------------------
# Release
# ------------------------------------------------------------------------------

# Tag and push a release; CI builds binaries and publishes the GitHub release
release version:
    @test -z "$(git status --porcelain)" || (echo "working tree is dirty"; exit 1)
    git tag "v{{version}}"
    git push origin "v{{version}}"
