# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

A Rust client library and CLI for interacting with BuildKit (moby/buildkit) to build container images via gRPC. Implements the complete BuildKit session protocol including bidirectional streaming, file synchronization, and HTTP/2-over-gRPC tunneling.

## Essential Commands

### Setup & Build
```bash
# First-time setup: initialize proto files
./scripts/init-proto.sh

# Start BuildKit daemon and local registry
docker-compose up -d

# Build project
cargo build --release

# Quick health check
cargo run -- health
```

### Development
```bash
# Build with logging
RUST_LOG=info cargo run -- local -c examples/test-dockerfile -t localhost:5000/test:latest

# Trace session protocol debugging
RUST_LOG=info,buildkit_client::session::grpc_tunnel=trace cargo run -- local -c . -t test:latest

# Session and fsutil protocol debugging
RUST_LOG=info,buildkit_client::session=debug timeout 25 cargo run -- local -c . -t test:latest
```

### Testing

#### Unit Tests (no BuildKit required)
```bash
# All unit tests
cargo test --lib
cargo test --test builder_test
cargo test --test session_test
cargo test --test progress_test

# Single test with output
cargo test test_platform_parse -- --nocapture
```

#### Integration Tests (requires BuildKit)
```bash
# Start BuildKit first
docker run -d --rm --privileged -p 1234:1234 moby/buildkit:latest --addr tcp://0.0.0.0:1234

# Run integration tests
cargo test --test integration_test -- --test-threads=1

# Run GitHub repository tests
GITHUB_TOKEN=your_token cargo test --test integration_test github -- --test-threads=1

# Using test script
./scripts/test.sh all          # All tests
./scripts/test.sh unit         # Unit tests only
./scripts/test.sh integration  # Integration tests
./scripts/test.sh github       # GitHub tests
```

#### Test Utilities (Makefile)
```bash
make -f Makefile.test test              # Unit tests
make -f Makefile.test test-integration  # Integration tests
make -f Makefile.test test-github       # GitHub tests
make -f Makefile.test coverage          # Coverage report
make -f Makefile.test bench             # Benchmarks
```

### Quality Checks
```bash
cargo fmt            # Format code
cargo clippy         # Lint
cargo bench          # Run benchmarks
```

## Architecture

### High-Level Data Flow

```
BuildKitClient.build(config)
  ↓
1. Create Session with UUID
2. Add FileSync service (for local builds)
3. Session.start() → Opens bidirectional gRPC stream
4. HTTP/2 tunnel starts inside session BytesMessage stream
  ↓
5. Prepare SolveRequest with:
   - Frontend: "dockerfile.v0"
   - Context: "input:session-{uuid}:context" (local) or git URL (GitHub)
   - Session metadata headers (X-Docker-Expose-Session-*)
  ↓
6. control.solve(request) → BuildKit begins build
  ↓
7. BuildKit calls back through HTTP/2 tunnel:
   - DiffCopy: Stream build context files
   - Credentials: Get registry auth
   - Health: Check session alive
  ↓
8. DiffCopy protocol:
   a. Server sends STAT packets (file metadata)
   b. Client sends REQ packets (requests file by ID)
   c. Server sends DATA packets (file content)
   d. Both send FIN when complete
  ↓
9. BuildKit completes build and pushes image
10. Return BuildResult with digest
```

### Critical Components

#### 1. Session Protocol (`src/session/mod.rs`)
- Orchestrates bidirectional gRPC stream with BuildKit
- Manages file sync and auth services
- Generates session metadata headers required by BuildKit
- **Key headers**: `X-Docker-Expose-Session-Uuid`, `X-Docker-Expose-Session-Grpc-Method`

#### 2. HTTP/2-over-gRPC Tunnel (`src/session/grpc_tunnel.rs`)
**Most complex component** - Implements a complete gRPC server inside a gRPC stream:

```
BuildKit Control.Session stream (outer)
  ↓ BytesMessage containing HTTP/2 frames
h2 server (inner)
  ↓ HTTP/2 frames → gRPC calls
Route to handlers:
  - /moby.filesync.v1.FileSync/DiffCopy → bidirectional stream
  - /moby.filesync.v1.Auth/Credentials → unary
  - /grpc.health.v1.Health/Check → unary
```

**gRPC Message Framing**:
- Each message: `[compression(1)] + [length(4 BE)] + [protobuf payload]`
- Must parse frame boundaries from buffered BytesMessage chunks
- Must add 5-byte prefix when sending responses

#### 3. DiffCopy File Sync (`src/session/grpc_tunnel.rs` lines 168-511)
Implements fsutil's bidirectional streaming protocol for file transfer:

**Server → Client (sending files):**
1. Walk directory tree recursively
2. Send STAT packet for each entry (file/dir) with sequential ID (0, 1, 2...)
3. Send final empty STAT packet (no stat field) to signal end of listing
4. Wait for REQ packets from client

**Client → Server (requesting files):**
1. Receive all STAT packets
2. Send REQ packet with file ID for files that need data
3. Receive DATA packets for the file
4. Empty DATA packet signals end of that file
5. Send FIN packet when done requesting

**Server → Client (responding to REQs):**
1. Receive REQ packet with ID
2. Send DATA packets in chunks
3. Send empty DATA packet (len=0) to signal file EOF
4. Continue until FIN received
5. Send FIN response

**Critical Protocol Details:**
- IDs are assigned to ALL entries (files + directories), not just files
- Root directory itself gets NO STAT packet, only its children
- `file_map` only stores files (not dirs) since only files have data
- Mode bits must be correct: `0o100644` (files), `0o040755` (dirs)
- Empty packets signal EOF, not FIN (FIN is for entire transfer)

#### 4. Nested Loop Exit Pattern

**Bug to avoid in bidirectional streams:**
```rust
loop {                              // Outer: read from HTTP/2 stream
    match request_stream.data() {
        Some(Ok(chunk)) => {
            while buffer.len() >= 5 {  // Inner: parse gRPC messages
                // Decode packet
                if PacketType::PacketFin {
                    received_fin = true;
                    break;  // ⚠️ Only exits inner while loop!
                }
            }
            // ✅ MUST check flag and break outer loop
            if received_fin { break; }
        }
    }
}
```
Without the flag check, outer loop continues waiting → timeout.

#### 5. Auth Protocol (`src/session/auth.rs`)
- `GetTokenAuthority`: Return error → BuildKit falls back
- `Credentials`: Return empty creds if no auth → BuildKit proceeds without auth
- `FetchToken`: Return empty token
- **Critical**: All unary responses need `grpc-status: 0` in trailers

### Solve Operation (`src/solve.rs`)

Prepares BuildKit solve request with:
- **Frontend**: Always `"dockerfile.v0"`
- **Context**:
  - Local: `"input:{session_shared_key}:context"`
  - GitHub: `"https://token@github.com/user/repo.git#branch"`
- **Frontend attrs**: build-arg:*, target, platform, filename, no-cache
- **Exporters**: Push to registry with tags
- **Cache**: Import/export for layer caching

Session metadata headers MUST be attached to solve request for BuildKit to recognize the session.

## Testing Architecture

### Test Organization
- **Unit tests**: 39 tests in `tests/` directory
- **Integration tests**: 14+ tests requiring BuildKit
- **Benchmarks**: Performance tests in `benches/`
- **Test utilities**: Shared helpers in `tests/common/`

### GitHub Repository Tests
Test repositories for integration testing:
- **Public**: `https://github.com/buildkit-rs/hello-world-public`
- **Private**: `https://github.com/buildkit-rs/hello-world-private`
- Default token: `ffffff`

Use `GITHUB_TOKEN` environment variable for custom token.

### Test Documentation
- **TESTING.md**: Comprehensive testing guide
- **tests/GITHUB_TESTS.md**: GitHub build tests details
- **GITHUB_TEST_QUICK_START.md**: Quick reference (中文)
- **TEST_SUMMARY.md**: Complete test system overview

## Common Issues

### "no active session" error
- Session metadata headers missing from SolveRequest
- Check `X-Docker-Expose-Session-*` headers present
- Verify `session.start()` completed before `solve()`

### "context canceled" / build timeout
- DiffCopy stream not closed after FIN
- Check `received_fin` flag in nested loops (see pattern above)
- Verify trailers sent with `grpc-status: 0`

### "invalid file request N" error
- File ID mismatch between STAT packets and file_map
- IDs must increment for ALL entries (files + dirs)
- file_map should only contain actual files

### Build completes but push fails
- Registry not running: `docker run -d -p 5000:5000 registry:2`
- Check BUILDKIT_REGISTRY_INSECURE for localhost

### Proto compilation errors
```bash
make proto-clean
./scripts/init-proto.sh
cargo clean && cargo build
```

## Protocol References

For extending session protocol, consult:
- BuildKit source: `github.com/moby/buildkit`
- fsutil reference: `github.com/tonistiigi/fsutil`
  - Especially `send.go` (lines 150-250) for DiffCopy server behavior
  - And `receive.go` for client behavior
- Proto definitions: `proto/moby/buildkit/` and `proto/fsutil/`

## Key Implementation Files

```
src/
├── solve.rs              # Build execution, prepares SolveRequest with session
├── session/
│   ├── mod.rs            # Session lifecycle & metadata generation
│   ├── grpc_tunnel.rs    # HTTP/2 server + DiffCopy handler (lines 168-511)
│   ├── filesync.rs       # FileSyncServer helper (basic, most logic in tunnel)
│   └── auth.rs           # AuthServer for registry credentials
tests/
├── common/mod.rs         # Test utilities (create_temp_dir, random_test_tag, etc)
├── integration_test.rs   # Full workflow tests including GitHub builds
└── GITHUB_TESTS.md       # GitHub test documentation
```

## Environment Variables

- `BUILDKIT_ADDR`: BuildKit address (default: `http://localhost:1234`)
- `GITHUB_TOKEN`: GitHub token for private repo tests
- `RUST_LOG`: Logging level (trace, debug, info, warn, error)
  - `RUST_LOG=info,buildkit_client::session::grpc_tunnel=trace` for protocol debugging

## Development Notes

- Proto files auto-generated from BuildKit repo via `scripts/init-proto.sh`
- Use `RUST_LOG=trace` for gRPC frame-level debugging
- h2 crate handles HTTP/2 framing; we manage request/response routing
- Session IDs must be UUID format; shared keys can be any unique string
- BuildKit requires specific file mode bits on STAT packets
- Tests use `--test-threads=1` to avoid BuildKit contention
