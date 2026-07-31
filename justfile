# List available recipes
default:
    @just --list

# Run all library tests
test:
    cargo test --lib

# Run clippy lints
lint:
    cargo clippy --all-targets -- -D warnings

# Run the GPUI desktop application
run:
    cargo run -p vector-editor-gpui

# Run the legacy wgpu desktop application
run-legacy:
    cargo run -p vector-editor-desktop

# Format code
format:
    cargo fmt
