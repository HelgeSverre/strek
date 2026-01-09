# Jakefile - Starter Template
# This is a starting point for your Jakefile. Customize as needed!

# Load environment variables from .env file
# (create a .env file with your variables)
@dotenv

# Export variables to child processes
# @export VARIABLE_NAME

# -----------------------------------------------------------------------------
# Tasks
# -----------------------------------------------------------------------------

# Default task - runs when you type just `jake`
task default: build

# -----------------------------------------------------------------------------
# Build
# -----------------------------------------------------------------------------

@group build
@desc "Build the project"
task build:
    # Add your build commands here
    echo "Building project..."
    # zig build
    # go build
    # cargo build
    # npm run build

# -----------------------------------------------------------------------------
# Clean
# -----------------------------------------------------------------------------

@group clean
@desc "Clean build artifacts"
task clean:
    # Add cleanup commands here
    echo "Cleaning..."
    # rm -rf dist/
    # rm -rf build/
    # cargo clean

# -----------------------------------------------------------------------------
# Development
# -----------------------------------------------------------------------------

@group dev
@desc "Development server with auto-rebuild"
task dev:
    # Watch for file changes and rebuild automatically
    # @watch src/**
    echo "Starting development mode..."

# -----------------------------------------------------------------------------
# Setup
# -----------------------------------------------------------------------------

@group setup
@desc "Install dependencies"
task setup:
    # Add your dependency installation commands here
    echo "Installing dependencies..."
    # npm install
    # cargo fetch
    # go mod download

# -----------------------------------------------------------------------------
# Test
# -----------------------------------------------------------------------------

@group test
@desc "Run tests"
task test:
    # Add your test commands here
    echo "Running tests..."
    # zig build test
    # go test ./...
    # cargo test
    # npm test
