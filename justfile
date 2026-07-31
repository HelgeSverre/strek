# List available recipes
default:
    @just --list

# Run all workspace tests
test:
    cargo nextest run --workspace --all-targets --locked

# Run clippy lints
lint:
    cargo clippy --all-targets -- -D warnings

# Run the GPUI desktop application
run:
    cargo run -p strek

# Format code
format:
    cargo fmt
