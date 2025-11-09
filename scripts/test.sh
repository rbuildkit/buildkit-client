#!/bin/bash
# Comprehensive test script for buildkit-client

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Print colored message
print_msg() {
    local color=$1
    shift
    echo -e "${color}$@${NC}"
}

print_header() {
    echo ""
    print_msg "$BLUE" "=================================="
    print_msg "$BLUE" "$1"
    print_msg "$BLUE" "=================================="
}

# Check if BuildKit is running
check_buildkit() {
    local addr=${BUILDKIT_ADDR:-http://localhost:1234}
    print_msg "$YELLOW" "Checking BuildKit at $addr..."

    if curl -f "$addr/healthz" &> /dev/null; then
        print_msg "$GREEN" "✓ BuildKit is running"
        return 0
    else
        print_msg "$RED" "✗ BuildKit is not running at $addr"
        return 1
    fi
}

# Start BuildKit using docker-compose
start_buildkit() {
    print_msg "$YELLOW" "Starting BuildKit..."

    if [ -f "docker-compose.yml" ]; then
        docker-compose up -d buildkit
        sleep 5

        # Wait for BuildKit to be ready
        for i in {1..30}; do
            if check_buildkit &> /dev/null; then
                print_msg "$GREEN" "✓ BuildKit started successfully"
                return 0
            fi
            echo -n "."
            sleep 1
        done

        print_msg "$RED" "✗ BuildKit failed to start"
        return 1
    else
        print_msg "$YELLOW" "No docker-compose.yml found. Starting BuildKit manually..."
        docker run -d --name buildkit-test --rm --privileged -p 1234:1234 \
            moby/buildkit:latest --addr tcp://0.0.0.0:1234

        sleep 5

        for i in {1..30}; do
            if check_buildkit &> /dev/null; then
                print_msg "$GREEN" "✓ BuildKit started successfully"
                return 0
            fi
            echo -n "."
            sleep 1
        done

        print_msg "$RED" "✗ BuildKit failed to start"
        return 1
    fi
}

# Run formatting check
run_fmt_check() {
    print_header "Checking formatting"
    if cargo fmt -- --check; then
        print_msg "$GREEN" "✓ Formatting check passed"
        return 0
    else
        print_msg "$RED" "✗ Formatting check failed"
        print_msg "$YELLOW" "Run 'cargo fmt' to fix formatting"
        return 1
    fi
}

# Run clippy
run_clippy() {
    print_header "Running clippy"
    if cargo clippy --all-targets --all-features -- -D warnings; then
        print_msg "$GREEN" "✓ Clippy passed"
        return 0
    else
        print_msg "$RED" "✗ Clippy found issues"
        return 1
    fi
}

# Run unit tests
run_unit_tests() {
    print_header "Running unit tests"

    print_msg "$YELLOW" "Running library tests..."
    cargo test --lib --verbose

    print_msg "$YELLOW" "Running builder tests..."
    cargo test --test builder_test --verbose

    print_msg "$YELLOW" "Running session tests..."
    cargo test --test session_test --verbose

    print_msg "$YELLOW" "Running progress tests..."
    cargo test --test progress_test --verbose

    print_msg "$YELLOW" "Running proto tests..."
    cargo test --test proto_test --verbose

    print_msg "$GREEN" "✓ All unit tests passed"
}

# Run integration tests
run_integration_tests() {
    print_header "Running integration tests"

    if ! check_buildkit; then
        print_msg "$YELLOW" "BuildKit not running. Attempting to start..."
        if ! start_buildkit; then
            print_msg "$RED" "Cannot run integration tests without BuildKit"
            return 1
        fi
    fi

    print_msg "$YELLOW" "Running integration tests..."
    cargo test --test integration_test --verbose -- --test-threads=1

    print_msg "$GREEN" "✓ All integration tests passed"
}

# Run GitHub integration tests
run_github_tests() {
    print_header "Running GitHub integration tests"

    if ! check_buildkit; then
        print_msg "$YELLOW" "BuildKit not running. Attempting to start..."
        if ! start_buildkit; then
            print_msg "$RED" "Cannot run GitHub tests without BuildKit"
            return 1
        fi
    fi

    # Check if GITHUB_TOKEN is set
    if [ -z "$GITHUB_TOKEN" ]; then
        print_msg "$YELLOW" "Warning: GITHUB_TOKEN not set. Private repository tests will use default token."
        print_msg "$YELLOW" "Set GITHUB_TOKEN environment variable for your own token."
    else
        print_msg "$GREEN" "Using GITHUB_TOKEN from environment"
    fi

    print_msg "$YELLOW" "Running GitHub repository tests..."
    cargo test --test integration_test github --verbose -- --test-threads=1

    print_msg "$GREEN" "✓ All GitHub tests passed"
}

# Run benchmarks
run_benchmarks() {
    print_header "Running benchmarks"

    cargo bench --bench build_bench

    print_msg "$GREEN" "✓ Benchmarks completed"
}

# Run all tests
run_all() {
    local failed=0

    run_fmt_check || failed=1
    run_clippy || failed=1
    run_unit_tests || failed=1
    run_integration_tests || failed=1

    if [ $failed -eq 0 ]; then
        print_header "All tests passed! 🎉"
        return 0
    else
        print_header "Some tests failed"
        return 1
    fi
}

# Generate coverage report
run_coverage() {
    print_header "Generating coverage report"

    if ! command -v cargo-tarpaulin &> /dev/null; then
        print_msg "$YELLOW" "Installing cargo-tarpaulin..."
        cargo install cargo-tarpaulin
    fi

    if ! check_buildkit; then
        print_msg "$YELLOW" "BuildKit not running. Attempting to start..."
        start_buildkit || exit 1
    fi

    cargo tarpaulin --verbose --all-features --workspace --timeout 300 \
        --out Html --output-dir coverage

    print_msg "$GREEN" "✓ Coverage report generated in coverage/index.html"

    if command -v open &> /dev/null; then
        open coverage/index.html
    elif command -v xdg-open &> /dev/null; then
        xdg-open coverage/index.html
    fi
}

# Clean test artifacts
clean() {
    print_header "Cleaning test artifacts"

    cargo clean

    # Clean temporary test directories
    rm -rf /tmp/buildkit-test-*

    # Stop BuildKit if running
    if docker ps | grep -q buildkit-test; then
        docker stop buildkit-test
    fi

    if [ -f "docker-compose.yml" ]; then
        docker-compose down
    fi

    print_msg "$GREEN" "✓ Cleaned"
}

# Show help
show_help() {
    cat << EOF
Usage: $0 [COMMAND]

Commands:
    all         Run all tests (format, clippy, unit, integration)
    fmt         Check code formatting
    clippy      Run clippy linter
    unit        Run unit tests only
    integration Run integration tests only (requires BuildKit)
    github      Run GitHub repository tests (requires BuildKit)
    bench       Run benchmarks
    coverage    Generate test coverage report
    buildkit    Start BuildKit daemon
    check       Check if BuildKit is running
    clean       Clean test artifacts and stop BuildKit
    help        Show this help message

Environment Variables:
    BUILDKIT_ADDR    BuildKit address (default: http://localhost:1234)
    GITHUB_TOKEN     GitHub personal access token for private repo tests
    RUST_LOG         Logging level (default: info)

Examples:
    $0 all                          # Run all tests
    $0 unit                         # Run only unit tests
    $0 integration                  # Run only integration tests
    $0 github                       # Run GitHub repository tests
    RUST_LOG=debug $0 integration   # Run with debug logging
    GITHUB_TOKEN=xxx $0 github      # Run GitHub tests with custom token
    $0 coverage                     # Generate coverage report

EOF
}

# Main script
main() {
    local command=${1:-all}

    case $command in
        all)
            run_all
            ;;
        fmt)
            run_fmt_check
            ;;
        clippy)
            run_clippy
            ;;
        unit)
            run_unit_tests
            ;;
        integration)
            run_integration_tests
            ;;
        github)
            run_github_tests
            ;;
        bench)
            run_benchmarks
            ;;
        coverage)
            run_coverage
            ;;
        buildkit)
            start_buildkit
            ;;
        check)
            check_buildkit
            ;;
        clean)
            clean
            ;;
        help|--help|-h)
            show_help
            ;;
        *)
            print_msg "$RED" "Unknown command: $command"
            echo ""
            show_help
            exit 1
            ;;
    esac
}

main "$@"
