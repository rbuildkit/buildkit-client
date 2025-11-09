# Testing Guide

Complete guide to testing buildkit-client, including unit tests, integration tests, GitHub repository builds, and benchmarks.

## Table of Contents

- [Quick Start](#quick-start)
- [Test Overview](#test-overview)
- [Running Tests](#running-tests)
- [GitHub Repository Tests](#github-repository-tests)
- [Test Structure](#test-structure)
- [Writing Tests](#writing-tests)
- [Configuration](#configuration)
- [Troubleshooting](#troubleshooting)
- [CI/CD Integration](#cicd-integration)

## Quick Start

### Prerequisites

1. **BuildKit Running**
   ```bash
   docker run -d --rm --privileged -p 1234:1234 moby/buildkit:latest --addr tcp://0.0.0.0:1234
   ```

2. **Network Connection** (for GitHub tests)

### Run All Tests

```bash
# Method 1: Using test script (recommended)
./scripts/test.sh all

# Method 2: Using Makefile
make -f Makefile.test test-all

# Method 3: Direct cargo
cargo test
```

### Run Specific Test Suites

```bash
# Unit tests only (no BuildKit required)
./scripts/test.sh unit
# or
cargo test --lib

# Integration tests (requires BuildKit)
./scripts/test.sh integration
# or
cargo test --test integration_test -- --test-threads=1

# GitHub repository tests
./scripts/test.sh github
# or
GITHUB_TOKEN=your_token cargo test --test integration_test github -- --test-threads=1

# Benchmarks
cargo bench
```

## Test Overview

### Test Statistics

- **Unit Tests**: 39 tests
- **Integration Tests**: 14+ tests (including GitHub builds)
- **Benchmarks**: 7 benchmark groups
- **Test Utilities**: 11+ helper functions

### Test Types

| Type | Count | Requires BuildKit | Description |
|------|-------|-------------------|-------------|
| Unit Tests | 39 | No | Test individual components |
| Integration Tests | 13 | Yes | Full workflow tests |
| GitHub Tests | 9 | Yes | Repository build tests |
| Benchmarks | 7 | No | Performance tests |

### Test Files

```
tests/
├── common/
│   └── mod.rs              # Shared utilities and fixtures
├── builder_test.rs         # BuildConfig, Platform tests (11 tests)
├── session_test.rs         # Session protocol tests (13 tests)
├── progress_test.rs        # Progress handler tests (14 tests)
├── proto_test.rs           # Proto compilation test (1 test)
└── integration_test.rs     # Full workflow tests (14 tests)

benches/
└── build_bench.rs          # Performance benchmarks
```

## Running Tests

### Unit Tests (No BuildKit Required)

Unit tests are fast and don't require external services:

```bash
# Run all unit tests
cargo test --lib
cargo test --test builder_test
cargo test --test session_test
cargo test --test progress_test

# Run specific test
cargo test test_platform_parse

# Run with output
cargo test test_platform_parse -- --nocapture

# Run with logging
RUST_LOG=debug cargo test
```

**What's tested:**
- ✅ Platform parsing and string conversion (11 tests)
- ✅ BuildConfig creation and builder pattern
- ✅ Session management and metadata
- ✅ Progress handlers (Console, JSON, Silent)
- ✅ Registry authentication
- ✅ Cache configuration

### Integration Tests (Requires BuildKit)

Integration tests verify full build workflows:

```bash
# Start BuildKit first
docker run -d --rm --privileged -p 1234:1234 moby/buildkit:latest --addr tcp://0.0.0.0:1234

# Run integration tests
cargo test --test integration_test -- --test-threads=1

# Run specific test
cargo test --test integration_test test_simple_local_build

# Run with custom BuildKit address
BUILDKIT_ADDR=http://localhost:5678 cargo test --test integration_test
```

**What's tested:**
- ✅ BuildKit connection and health check
- ✅ Simple local builds
- ✅ Builds with custom Dockerfile paths
- ✅ Builds with build arguments
- ✅ Multi-stage builds with target
- ✅ Builds with .dockerignore
- ✅ Progress handler integration
- ✅ Error handling scenarios

### Benchmarks

Performance benchmarks use Criterion:

```bash
# Run all benchmarks
cargo bench

# Run specific benchmark
cargo bench platform_parse

# Save baseline for comparison
cargo bench -- --save-baseline my-baseline

# Compare against baseline
cargo bench -- --baseline my-baseline
```

**What's benchmarked:**
- Platform parsing and conversion
- BuildConfig creation
- Session metadata generation
- Dockerfile source matching
- Build args HashMap operations

## GitHub Repository Tests

### Test Repositories

#### Public Repository
- **URL**: `https://github.com/buildkit-rs/hello-world-public`
- **Access**: No authentication required
- **Purpose**: Test public repository builds

#### Private Repository
- **URL**: `https://github.com/buildkit-rs/hello-world-private`
- **Access**: Requires GitHub token
- **Default Token**: `*` (use `GITHUB_TOKEN` env var for custom token)
- **Purpose**: Test private repository authentication

### Running GitHub Tests

```bash
# Method 1: Using test script (recommended)
./scripts/test.sh github

# Method 2: Using Makefile
make -f Makefile.test test-github

# Method 3: Direct cargo
cargo test --test integration_test github -- --test-threads=1

# With custom token
GITHUB_TOKEN=your_token ./scripts/test.sh github

# With verbose output
RUST_LOG=info cargo test --test integration_test github -- --nocapture --test-threads=1
```

### GitHub Test Cases

**Public Repository Tests (7 tests):**
1. ✅ `test_github_public_repo_build` - Basic public repository build
2. ✅ `test_github_public_repo_with_ref` - Build with specific git branch
3. ✅ `test_github_with_custom_dockerfile` - Custom Dockerfile path
4. ✅ `test_github_with_build_args` - Build with arguments
5. ✅ `test_github_with_progress_handler` - Build with progress monitoring
6. ⏸️ `test_github_with_commit_ref` - Build with specific commit (ignored)
7. ✅ `test_github_private_without_token` - Access private repo without token (should fail)

**Private Repository Tests (2 tests):**
1. ✅ `test_github_private_repo_build` - Basic private repository build
2. ✅ `test_github_private_repo_with_ref` - Build with specific branch

### GitHub Token Configuration

Set via environment variable:

```bash
export GITHUB_TOKEN=your_token_here
cargo test --test integration_test github
```

**Creating a GitHub Token:**
1. Go to GitHub Settings → Developer settings → Personal access tokens
2. Generate new token (classic)
3. Select scope: `repo` (Full control of private repositories)
4. Copy the token and set as environment variable

## Test Structure

### Unit Test Example

```rust
#[test]
fn test_platform_parse() {
    let platform = Platform::parse("linux/amd64").unwrap();
    assert_eq!(platform.os, "linux");
    assert_eq!(platform.arch, "amd64");
}
```

### Integration Test Example

```rust
#[tokio::test]
async fn test_simple_build() {
    skip_without_buildkit!();  // Skip if BuildKit not available

    let test_dir = create_temp_dir("my-test");
    create_test_dockerfile(&test_dir, None);

    let mut client = BuildKitClient::connect(&get_buildkit_addr()).await.unwrap();
    let config = BuildConfig::local(&test_dir).tag(random_test_tag());

    let result = client.build(config, None).await;

    cleanup_temp_dir(&test_dir);
    assert!(result.is_ok());
}
```

## Writing Tests

### Using Test Utilities

The `common` module provides helpful utilities:

```rust
use common::*;

// Create temporary test directory
let dir = create_temp_dir("test-name");

// Create test Dockerfiles
create_test_dockerfile(&dir, None);
create_multistage_dockerfile(&dir);
create_dockerfile_with_args(&dir);

// Create full test context with files
create_test_context(&dir);

// Generate random tag for testing
let tag = random_test_tag();

// Check if BuildKit is available
if !is_buildkit_available().await {
    return; // Skip test
}

// Use macro to skip test
skip_without_buildkit!();

// Cleanup
cleanup_temp_dir(&dir);
```

### Test Utilities Reference

| Function | Description |
|----------|-------------|
| `get_buildkit_addr()` | Get BuildKit address from environment |
| `is_buildkit_available()` | Check if BuildKit is running |
| `create_temp_dir(name)` | Create temporary test directory |
| `create_test_dockerfile(dir, content)` | Create simple test Dockerfile |
| `create_multistage_dockerfile(dir)` | Create multi-stage Dockerfile |
| `create_dockerfile_with_args(dir)` | Create Dockerfile with build args |
| `create_test_context(dir)` | Create full test context with files |
| `create_dockerignore(dir)` | Create .dockerignore file |
| `random_test_tag()` | Generate random test image tag |
| `cleanup_temp_dir(dir)` | Clean up temporary directory |
| `skip_without_buildkit!()` | Macro to skip tests without BuildKit |

### Test Naming Conventions

- Unit tests: `test_<component>_<behavior>`
- Integration tests: `test_<workflow>_<scenario>`
- Benchmarks: `bench_<operation>_<variant>`

Examples:
- `test_platform_parse_with_variant`
- `test_build_with_custom_dockerfile`
- `bench_session_metadata_generation`

## Configuration

### Environment Variables

| Variable | Description | Default | Required |
|----------|-------------|---------|----------|
| `BUILDKIT_ADDR` | BuildKit service address | `http://localhost:1234` | For integration tests |
| `GITHUB_TOKEN` | GitHub access token | Built-in test token | For private repo tests |
| `RUST_LOG` | Log level | `info` | No |

### Examples

```bash
# Custom BuildKit address
BUILDKIT_ADDR=http://localhost:5678 cargo test

# With debug logging
RUST_LOG=debug cargo test

# GitHub tests with custom token
GITHUB_TOKEN=your_token cargo test --test integration_test github

# Combined
RUST_LOG=debug BUILDKIT_ADDR=http://localhost:1234 cargo test
```

### Test Isolation

Integration tests should run with `--test-threads=1` to avoid conflicts:

```bash
cargo test --test integration_test -- --test-threads=1
```

## Troubleshooting

### BuildKit Connection Failed

**Problem**: Tests fail with "connection refused"

**Solution**:
```bash
# Check if BuildKit is running
docker ps | grep buildkit

# Restart BuildKit
docker stop $(docker ps -q --filter ancestor=moby/buildkit)
docker run -d --rm --privileged -p 1234:1234 moby/buildkit:latest --addr tcp://0.0.0.0:1234

# Or use the test script
./scripts/test.sh buildkit

# Verify health
curl http://localhost:1234/healthz
```

### Tests Timing Out

**Problem**: Integration tests timeout

**Solution**:
- Increase test timeout: `cargo test -- --test-timeout 300`
- Check BuildKit health: `curl http://localhost:1234/healthz`
- Ensure BuildKit has sufficient resources
- Check BuildKit logs: `docker logs <container_id>`

### Private Repository Authentication Failed

**Problem**: GitHub private repository tests fail

**Solution**:
```bash
# Set correct GitHub token
export GITHUB_TOKEN=*

# Verify token works
curl -H "Authorization: token $GITHUB_TOKEN" https://api.github.com/user

# Check token permissions include 'repo' scope
```

### File Permission Errors

**Problem**: Test fails with permission denied

**Solution**:
```bash
# Cleanup test directories
rm -rf /tmp/buildkit-test-*

# Ensure proper permissions
chmod -R u+w /tmp/buildkit-test-* 2>/dev/null || true
```

### Port Already in Use

**Problem**: BuildKit port 1234 already in use

**Solution**:
```bash
# Use different port
docker run -d --rm --privileged -p 5678:5678 moby/buildkit:latest --addr tcp://0.0.0.0:5678

# Set environment variable
BUILDKIT_ADDR=http://localhost:5678 cargo test
```

### Proto Compilation Errors

**Problem**: Tests fail with proto errors

**Solution**:
```bash
# Reinitialize proto files
./scripts/init-proto.sh

# Clean and rebuild
cargo clean
cargo build
```

### Network Timeout (GitHub Tests)

**Problem**: GitHub tests timeout

**Solution**:
```bash
# Check network connection
curl -I https://github.com

# Use proxy if needed
export HTTP_PROXY=http://your-proxy:port
export HTTPS_PROXY=http://your-proxy:port

# Increase timeout in test
```

## CI/CD Integration

### GitHub Actions

The project includes a GitHub Actions workflow (`.github/workflows/test.yml`) that:

1. Runs on push to main/master/develop branches
2. Runs on pull requests
3. Tests on multiple Rust versions (stable, beta, nightly)
4. Starts BuildKit service
5. Runs all test suites
6. Generates coverage reports
7. Runs benchmarks (on main branch)
8. Performs security audits

**Example workflow:**

```yaml
name: Tests

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    services:
      buildkit:
        image: moby/buildkit:latest
        ports:
          - 1234:1234
        options: --privileged

    steps:
    - uses: actions/checkout@v4

    - name: Setup Rust
      uses: dtolnay/rust-toolchain@stable

    - name: Run tests
      env:
        BUILDKIT_ADDR: http://localhost:1234
        GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
      run: |
        cargo test --all-features
        cargo test --test integration_test -- --test-threads=1
```

### Local CI Simulation

Simulate CI environment locally:

```bash
# Run all checks like CI does
./scripts/test.sh all

# Or manually:
cargo fmt -- --check
cargo clippy -- -D warnings
cargo test --all-features
cargo test --test integration_test -- --test-threads=1

# Using Makefile
make -f Makefile.test ci
```

### Test Coverage

Generate test coverage report:

```bash
# Install tarpaulin
cargo install cargo-tarpaulin

# Generate HTML coverage report
cargo tarpaulin --out Html --output-dir coverage

# Open coverage report
open coverage/index.html

# Or use the script
./scripts/test.sh coverage
```

### Watch Mode

Use `cargo-watch` for continuous testing:

```bash
# Install cargo-watch
cargo install cargo-watch

# Run tests on file changes
cargo watch -x test

# Run specific test suite
cargo watch -x "test --test builder_test"
```

## Performance Tips

- **GitHub tests**: First run takes 2-5 minutes (cloning + pulling images), subsequent runs use BuildKit cache (~30s-1min)
- **Parallel execution**: Use `--test-threads=1` for integration tests to avoid resource conflicts
- **BuildKit caching**: BuildKit caches Git repositories and Docker layers automatically
- **Incremental builds**: Cargo's incremental compilation speeds up test iterations

## Security Notes

⚠️ **Important Security Considerations:**

1. **Never commit GitHub tokens to version control**
2. Use environment variables for sensitive data
3. Rotate tokens regularly
4. Use tokens with minimal required permissions
5. Consider using fine-grained personal access tokens
6. In CI/CD, use secrets management (e.g., GitHub Secrets)
7. Audit token usage logs regularly

## Test Maintenance

### Updating Test Fixtures

When updating test fixtures:

1. Update the fixture in `tests/common/mod.rs`
2. Run all affected tests
3. Update documentation if behavior changes

### Adding New Tests

When adding new tests:

1. Choose appropriate test type (unit/integration)
2. Use existing utilities from `common` module
3. Add cleanup code for temporary resources
4. Document any special requirements
5. Update this documentation if needed

### Running Tests Before Commits

```bash
# Quick pre-commit check
cargo fmt
cargo clippy
cargo test --lib

# Full pre-commit check
./scripts/test.sh all
```

## Appendix

### Complete Test Command Reference

```bash
# Unit Tests
cargo test --lib
cargo test --test builder_test
cargo test --test session_test
cargo test --test progress_test
cargo test --test proto_test

# Integration Tests
cargo test --test integration_test -- --test-threads=1

# GitHub Tests
cargo test --test integration_test github -- --test-threads=1

# Benchmarks
cargo bench

# Coverage
cargo tarpaulin --out Html

# With scripts
./scripts/test.sh all          # All tests
./scripts/test.sh unit         # Unit tests
./scripts/test.sh integration  # Integration tests
./scripts/test.sh github       # GitHub tests
./scripts/test.sh coverage     # Coverage report
./scripts/test.sh buildkit     # Start BuildKit
./scripts/test.sh clean        # Clean artifacts

# With Makefile
make -f Makefile.test test              # Unit tests
make -f Makefile.test test-integration  # Integration tests
make -f Makefile.test test-github       # GitHub tests
make -f Makefile.test test-all          # All tests
make -f Makefile.test coverage          # Coverage
make -f Makefile.test bench             # Benchmarks
make -f Makefile.test ci                # CI checks
```

---

**Need Help?** Check the [main README](../README.md) or open an issue on GitHub.
