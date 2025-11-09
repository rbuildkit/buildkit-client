# Quick Start Guide

## First-Time Setup (3 Steps)

### 1. Initialize Proto Files

```bash
./scripts/init-proto.sh
```

Or using Makefile:

```bash
make proto-init
```

### 2. Start BuildKit and Registry

```bash
docker-compose up -d
# or
make up
```

### 3. Build and Test

```bash
cargo build
cargo run -- health
# or
make build
make health
```

## Common Commands

### Proto File Management

```bash
# Initialize (first time use)
make proto-init

# Update to latest version
make proto-clean
make proto-init
```

### Development

```bash
# View all available commands
make help

# Build
make build

# Run tests
make test

# Code checks
make check
make fmt
make clippy
```

### Docker Management

```bash
# Start services
make up

# Stop services
make down

# View logs
make logs
```

### Test Builds

```bash
# Health check
make health

# Test local build
make run-local

# Test GitHub build
make run-github
```

## Project Structure

```
buildkit-client/
├── scripts/
│   └── init-proto.sh      # Proto file initialization script
├── proto/                  # Proto files (auto-generated, in .gitignore)
├── src/                    # Source code
├── tests/                  # Tests
├── examples/               # Sample Dockerfiles
├── docs/                   # Documentation
├── Makefile               # Common commands
└── README.md              # Complete documentation
```

## Troubleshooting

### Proto File Issues

```bash
# Complete reset
rm -rf proto
./scripts/init-proto.sh
cargo clean
cargo build
```

### BuildKit Connection Issues

```bash
# Check service status
docker-compose ps

# Restart services
make down
make up

# View logs
make logs
```

### Compilation Issues

```bash
# Clean and rebuild
cargo clean
cargo build

# Or using Makefile
make clean
make build
```

## Next Steps

- Read [README.md](../README.md) for complete features
- Check [PROTO_SETUP.md](./PROTO_SETUP.md) for proto management details
- Explore `examples/` directory for sample Dockerfiles
- Run `make help` to view all available commands
