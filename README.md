# BuildKit Rust Client

<div align="center">

[![Crates.io](https://img.shields.io/crates/v/buildkit-client?style=flat-square)](https://crates.io/crates/buildkit-client)
[![Documentation](https://img.shields.io/docsrs/buildkit-client?style=flat-square)](https://docs.rs/buildkit-client)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue?style=flat-square)](#license)
[![Rust Version](https://img.shields.io/badge/rust-1.70%2B-orange?style=flat-square)](https://www.rust-lang.org)
[![Build Status](https://img.shields.io/github/actions/workflow/status/corespeed-io/buildkit-client/ci.yml?style=flat-square)](https://github.com/corespeed-io/buildkit-client/actions)
[![codecov](https://img.shields.io/codecov/c/github/corespeed-io/buildkit-client?style=flat-square)](https://codecov.io/gh/corespeed-io/buildkit-client)

A full-featured Rust client library and CLI for interacting with [moby/buildkit](https://github.com/moby/buildkit) to build container images via gRPC.

[Features](#features) •
[Installation](#installation) •
[Quick Start](#quick-start) •
[Usage](#usage) •
[Documentation](#documentation) •
[Contributing](#contributing)

</div>

---

## Features

- ✅ **Complete gRPC Implementation** - Direct integration with BuildKit's gRPC API
- 🏗️ **Multiple Build Sources** - Support for local Dockerfiles and GitHub repositories
- 🔐 **Authentication Support** - GitHub private repositories and Docker Registry authentication
- 🚀 **Advanced Build Options** - Build args, target stages, multi-platform builds
- 📊 **Real-time Progress** - Live build progress and log streaming
- 💾 **Cache Management** - Support for cache import/export
- 🎯 **Registry Push** - Automatic push of built images to registries
- 🔄 **Session Protocol** - Full implementation of BuildKit's bidirectional session protocol
- 🌐 **HTTP/2 Tunneling** - HTTP/2-over-gRPC for file synchronization

## Prerequisites

- Rust 1.70+
- Docker or BuildKit daemon
- Git (for fetching proto files)

## Installation

### As a Library

Add to your `Cargo.toml`:

```toml
[dependencies]
buildkit-client = "0.1"
tokio = { version = "1", features = ["full"] }
anyhow = "1.0"
```

### As a CLI Tool

```bash
git clone https://github.com/corespeed-io/buildkit-client.git
cd buildkit-client
./scripts/init-proto.sh
cargo install --path .
```

## Quick Start

### 0. Initialize Proto Files

First-time setup requires fetching protobuf definitions:

```bash
./scripts/init-proto.sh
```

> For detailed instructions, see [docs/PROTO_SETUP.md](./docs/PROTO_SETUP.md)

### 1. Start BuildKit and Registry

```bash
docker-compose up -d
```

This starts:
- BuildKit daemon (port 1234)
- Local Docker Registry (port 5000)

### 2. Build the Project

```bash
cargo build --release
```

### 3. Run Examples

#### Health Check

```bash
cargo run -- health
```

#### Build Local Dockerfile

```bash
cargo run -- local \
  --context ./examples/test-dockerfile \
  --tag localhost:5000/test:latest
```

#### Using Build Arguments

```bash
cargo run -- local \
  --context ./examples/multi-stage \
  --tag localhost:5000/multi-stage:latest \
  --build-arg APP_VERSION=2.0.0 \
  --build-arg BUILD_DATE=$(date +%Y-%m-%d)
```

#### Specify Target Stage

```bash
cargo run -- local \
  --context ./examples/multi-stage \
  --tag localhost:5000/dev:latest \
  --target dev
```

#### Multi-platform Build

```bash
cargo run -- local \
  --context ./examples/test-dockerfile \
  --tag localhost:5000/multi-arch:latest \
  --platform linux/amd64 \
  --platform linux/arm64
```

#### Build from GitHub Repository

```bash
# Public repository
cargo run -- github https://github.com/user/repo.git \
  --tag localhost:5000/from-github:latest \
  --git-ref main

# Private repository (with environment variable)
export GITHUB_TOKEN=ghp_your_token_here
cargo run -- github https://github.com/user/private-repo.git \
  --tag localhost:5000/private:latest \
  --git-ref main
```

#### Build with Registry Authentication

```bash
cargo run -- local \
  --context ./examples/test-dockerfile \
  --tag registry.example.com/myapp:latest \
  --registry-host registry.example.com \
  --registry-user myuser \
  --registry-password mypassword
```

#### JSON Output Mode

```bash
cargo run -- local \
  --context ./examples/test-dockerfile \
  --tag localhost:5000/test:latest \
  --json
```

## Usage

### Basic Example

```rust
use buildkit_client::{BuildKitClient, BuildConfig};
use buildkit_client::progress::ConsoleProgressHandler;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Connect to BuildKit
    let mut client = BuildKitClient::connect("http://localhost:1234").await?;

    // Configure build
    let config = BuildConfig::local("./my-app")
        .tag("localhost:5000/my-app:latest")
        .build_arg("VERSION", "1.0.0");

    // Execute build
    let progress = Box::new(ConsoleProgressHandler::new(true));
    let result = client.build(config, Some(progress)).await?;

    println!("✅ Build completed!");
    if let Some(digest) = result.digest {
        println!("📦 Image digest: {}", digest);
    }

    Ok(())
}
```

### GitHub Repository Build

```rust
use buildkit_client::{BuildKitClient, BuildConfig, RegistryAuth};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut client = BuildKitClient::connect("http://localhost:1234").await?;

    let config = BuildConfig::github("https://github.com/user/repo.git")
        .git_ref("main")
        .github_token("ghp_your_token")
        .dockerfile("path/to/Dockerfile")
        .tag("localhost:5000/from-github:latest")
        .build_arg("ENV", "production");

    let result = client.build(config, None).await?;
    Ok(())
}
```

### Multi-platform Build

```rust
use buildkit_client::{BuildKitClient, BuildConfig, Platform};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut client = BuildKitClient::connect("http://localhost:1234").await?;

    let config = BuildConfig::local("./my-app")
        .tag("localhost:5000/multi-arch:latest")
        .platform(Platform::linux_amd64())
        .platform(Platform::linux_arm64())
        .platform(Platform::parse("linux/arm/v7")?);

    let result = client.build(config, None).await?;
    Ok(())
}
```

## Project Structure

```
buildkit-client/
├── src/
│   ├── main.rs          # CLI tool entry point
│   ├── lib.rs           # Library entry point
│   ├── client.rs        # BuildKit gRPC client
│   ├── builder.rs       # Build configuration
│   ├── solve.rs         # Build execution logic
│   ├── progress.rs      # Progress handling
│   ├── session/         # Session protocol implementation
│   │   ├── mod.rs       # Session lifecycle & metadata
│   │   ├── grpc_tunnel.rs  # HTTP/2-over-gRPC tunnel
│   │   ├── filesync.rs  # File synchronization
│   │   └── auth.rs      # Registry authentication
│   └── proto.rs         # Protobuf generated code
├── proto/               # BuildKit protobuf definitions
├── examples/            # Sample Dockerfiles
├── tests/               # Comprehensive test suite
├── docker-compose.yml   # Test environment setup
└── README.md
```

## BuildKit gRPC API

This project directly uses BuildKit's gRPC API:

- `Control.Solve` - Execute build operations
- `Control.Status` - Stream build status updates
- `Control.Info` - Get BuildKit information
- `Control.Session` - Bidirectional session stream

All protobuf definitions are fetched from the [moby/buildkit](https://github.com/moby/buildkit) repository.

## Configuration Options

### BuildConfig

- `source` - Build source (local or GitHub)
- `dockerfile_path` - Path to Dockerfile
- `build_args` - Build arguments
- `target` - Target stage
- `platforms` - List of target platforms
- `tags` - List of image tags
- `registry_auth` - Registry authentication info
- `cache_from` - Cache import sources
- `cache_to` - Cache export destinations
- `secrets` - Build-time secrets
- `no_cache` - Disable caching
- `pull` - Always pull base images

### ProgressHandler

Three progress handlers are provided:

1. **ConsoleProgressHandler** - Output to console with colors
2. **JsonProgressHandler** - JSON format output
3. **SilentProgressHandler** - Silent mode

## Environment Variables

- `BUILDKIT_ADDR` - BuildKit address (default: `http://localhost:1234`)
- `GITHUB_TOKEN` - GitHub authentication token
- `RUST_LOG` - Log level (trace, debug, info, warn, error)
  - `RUST_LOG=info,buildkit_client::session::grpc_tunnel=trace` for protocol debugging

## Documentation

- **[Quick Start Guide](./docs/QUICK_START.md)** - Get up and running quickly
- **[Proto Setup](./docs/PROTO_SETUP.md)** - Proto file management
- **[Testing Guide](./docs/TESTING.md)** - Complete testing documentation (unit, integration, GitHub builds)
- **[Development Guide](./CLAUDE.md)** - Architecture and development guide

## Troubleshooting

### BuildKit Connection Failed

```bash
# Check if BuildKit is running
docker-compose ps

# View BuildKit logs
docker-compose logs buildkitd

# Restart services
docker-compose restart
```

### Registry Push Failed

Ensure the registry allows insecure connections (for localhost):

```yaml
# docker-compose.yml
services:
  buildkitd:
    environment:
      - BUILDKIT_REGISTRY_INSECURE=true
```

### Proto Compilation Errors

If you encounter protobuf compilation errors:

```bash
# Clean proto files and reinitialize
make proto-clean
make proto-init

# Or manually
rm -rf proto
./scripts/init-proto.sh

# Clean and rebuild
cargo clean
cargo build
```

## Development

### Using Makefile

The project provides a Makefile to simplify common operations:

```bash
make help          # Show all available commands
make init          # Initialize project (fetch proto and build)
make build         # Build project
make test          # Run tests
make up            # Start docker-compose services
make down          # Stop docker-compose services
make health        # Check BuildKit health status
```

### Testing

```bash
# Unit tests
cargo test --lib

# Integration tests (requires BuildKit)
cargo test --test integration_test -- --test-threads=1

# All tests
./scripts/test.sh all

# GitHub repository tests
GITHUB_TOKEN=your_token cargo test --test integration_test github -- --test-threads=1
```

### Update Protobuf Definitions

Proto files are automatically managed via scripts:

```bash
# Method 1: Using Makefile
make proto-clean
make proto-init

# Method 2: Manual execution
rm -rf proto
./scripts/init-proto.sh

# Rebuild
cargo build
```

### Code Formatting

```bash
cargo fmt
cargo clippy
```

### Benchmarks

```bash
cargo bench
```

## Architecture Highlights

### Session Protocol

Implements BuildKit's complete session protocol with:
- Bidirectional gRPC streaming
- HTTP/2-over-gRPC tunneling for callbacks
- File synchronization (DiffCopy protocol)
- Registry authentication

### HTTP/2 Tunnel

The HTTP/2-over-gRPC tunnel (`src/session/grpc_tunnel.rs`) is the most complex component:
- Runs a complete gRPC server inside a gRPC stream
- Routes incoming calls to appropriate handlers
- Implements proper gRPC message framing

### DiffCopy Protocol

Bidirectional file synchronization protocol:
- Server sends STAT packets (file metadata)
- Client sends REQ packets (file requests)
- Server sends DATA packets (file contents)
- Both send FIN when complete

For detailed architecture documentation, see [CLAUDE.md](./CLAUDE.md).

## License

This project is dual-licensed under MIT OR Apache-2.0.

## Acknowledgments

- [moby/buildkit](https://github.com/moby/buildkit) - BuildKit project
- [tonic](https://github.com/hyperium/tonic) - Rust gRPC library
- [prost](https://github.com/tokio-rs/prost) - Protocol Buffers implementation
- [h2](https://github.com/hyperium/h2) - HTTP/2 implementation

## Contributing

Contributions are welcome! Please feel free to submit Issues and Pull Requests.

Before submitting a PR:
1. Run `cargo fmt` and `cargo clippy`
2. Ensure all tests pass: `cargo test`
3. Add tests for new features
4. Update documentation as needed

## Related Links

- [BuildKit Documentation](https://github.com/moby/buildkit/tree/master/docs)
- [BuildKit API Reference](https://github.com/moby/buildkit/tree/master/api)
- [Docker Buildx](https://github.com/docker/buildx)
- [Container Image Specification](https://github.com/opencontainers/image-spec)

---

<div align="center">

**[⬆ back to top](#buildkit-rust-client)**

Made with ❤️ by AprilNEA

</div>
