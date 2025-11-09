# Proto File Management

This project uses automated scripts to manage protobuf files, eliminating the need for manual copying.

## Quick Start

### First-Time Setup

Run the initialization script to automatically fetch proto files:

```bash
./scripts/init-proto.sh
```

This script will:
1. Fetch the latest proto files from the [moby/buildkit](https://github.com/moby/buildkit) repository
2. Fetch dependent google/rpc proto files from [googleapis](https://github.com/googleapis/googleapis)
3. Copy files to the `proto/` directory

### Building the Project

After initializing proto files, build directly:

```bash
cargo build
```

The `build.rs` script automatically detects if proto files exist. If not found, it will attempt to run the initialization script.

## Updating Proto Files

If you need to update to the latest BuildKit proto definitions:

```bash
# Remove cached git clones
rm -rf proto/.buildkit proto/.googleapis

# Re-run initialization script
./scripts/init-proto.sh
```

## Git Ignore

The `proto/` directory is added to `.gitignore` because:
- Proto files are automatically managed via script
- Reduces repository size
- Always uses upstream latest definitions

Temporary git clone directories are also ignored:
- `proto/.buildkit/`
- `proto/.googleapis/`

## Manual Management (Optional)

If you prefer to manually manage proto files without using the script:

1. Clone from https://github.com/moby/buildkit and copy the following directories to `proto/`:
   - `api/`
   - `solver/`
   - `sourcepolicy/`
   - `frontend/`

2. Copy from https://github.com/googleapis/googleapis:
   - `google/rpc/*.proto` to `proto/google/rpc/`

3. Run `cargo build`

## Troubleshooting

### Proto Files Not Found

If the build reports proto files not found:

```bash
./scripts/init-proto.sh
cargo clean
cargo build
```

### Permission Issues

If the script cannot execute:

```bash
chmod +x scripts/init-proto.sh
./scripts/init-proto.sh
```

### Network Issues

If git clone fails (network issues), you can:
1. Manually download [buildkit](https://github.com/moby/buildkit/archive/refs/heads/master.zip)
2. Extract and copy the corresponding proto files to the `proto/` directory
