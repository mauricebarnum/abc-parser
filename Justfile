# Build, test, lint, document, run, and install the abc-parser workspace.
#
# Extra arguments are forwarded to the underlying command, for example:
#   just test -p abc-parser
#   just example kitchen_sink test_kitchen_sink.abc

set shell := ["bash", "-euo", "pipefail", "-c"]

# Options shared by workspace-wide Cargo commands.
cargo_flags := "--workspace --all-features"

# rustfmt.toml uses unstable options, so formatting requires nightly.
nightly := "+nightly"

# Treat warnings as errors, matching CI and pre-commit.
export CARGO_BUILD_WARNINGS := "deny"

# List available recipes.
default:
    @just --list

# Build every workspace member with debug information.
build *args:
    cargo build {{ cargo_flags }} {{ args }}

# Build optimized release binaries for every workspace member.
build-release *args:
    cargo build --release {{ cargo_flags }} {{ args }}

# Run unit and integration tests with Nextest.
test *args:
    cargo nextest run {{ cargo_flags }} {{ args }}

# Run documentation tests, which Nextest does not support.
test-doc *args:
    cargo test --doc {{ cargo_flags }} {{ args }}

# Run both Nextest suites and documentation tests.
test-all: test test-doc

# Check Rust formatting without modifying files.
fmt-check *args:
    cargo {{ nightly }} fmt --all -- --check {{ args }}

# Format the workspace in place with nightly rustfmt.
fmt *args:
    cargo {{ nightly }} fmt --all {{ args }}

# Run Clippy over all targets and features, treating warnings as errors.
clippy *args:
    cargo clippy --workspace --all-targets --all-features {{ args }}

# Run all cargo-deny checks: advisories, bans, licenses, and sources.
deny *args:
    cargo deny check {{ args }}

# Run every check CI performs.
check: fmt-check clippy test-all deny

# Build documentation and open it in the default browser.
doc *args:
    cargo doc --no-deps {{ cargo_flags }} --open {{ args }}

# Run a parser example, e.g. `just example kitchen_sink test_kitchen_sink.abc`.
example name *args:
    cargo run -p abc-parser --example {{ name }} -- {{ args }}

# Run the `abc-lint` binary.
run-lint *args:
    cargo run -p abc-lint -- {{ args }}

# Run the `abc-transpose` binary.
run-transpose *args:
    cargo run -p abc-transpose -- {{ args }}

# Run the ABC language server over standard input and output.
run-lsp *args:
    cargo run -p abc-language-server --bin abc-language-server -- {{ args }}

# Install `abc-lint` into ~/.cargo/bin.
install-lint *args:
    cargo install --path abc-lint --locked --force {{ args }}

# Install `abc-transpose` into ~/.cargo/bin.
install-transpose *args:
    cargo install --path abc-transpose --locked --force {{ args }}

# Install `abc-lint` and `abc-transpose` into ~/.cargo/bin.
install-parser: install-lint install-transpose

# Install `abc-language-server` into ~/.cargo/bin.
install-lsp *args:
    cargo install --path abc-language-server --bins --locked --force {{ args }}

# Install every workspace binary into ~/.cargo/bin.
install: install-lint install-transpose install-lsp

# Remove build artifacts.
clean:
    cargo clean

# Update dependencies within the semver ranges in Cargo.toml.
update:
    cargo update
