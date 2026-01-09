# List available recipes
default:
    @just --list

# Run all library tests
test:
    cargo test --lib

# Run clippy lints
lint:
    cargo clippy --all-targets -- -D warnings

# Build the web app WASM module (requires wasm-pack)
web-build:
    cd apps/web && wasm-pack build --target web --out-name vector_editor_web

# Serve the web app locally (requires bun)
web-serve:
    cd apps/web && bun run serve.ts

# Build and serve the web app
web port="3333": web-build
    @echo "Starting server at http://localhost:{{port}}"
    cd apps/web && bun run serve.ts {{port}}

# Run web app Playwright tests
web-test: web-build
    cd apps/web && bunx playwright test

# Run the desktop application
run:
    cargo run --bin vector-editor

# Format code
format:
    cargo fmt
