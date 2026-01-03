# QaOS Docker Quick Start

## Prerequisites
- Docker Desktop installed and running
- WSL2 backend enabled (for Windows)

## Quick Start

### 1. Build Docker Image
```bash
docker-compose build
```

### 2. Run QaOS (Interactive Shell)
```bash
docker-compose run --rm qaos bash
```

Inside container:
```bash
# Build the OS
cargo build -p os --target x86_64-unknown-none

# Run in QEMU (headless)
cargo xtask run
```

### 3. One-Command Build + Run
```bash
docker-compose up
```

## Development Workflow

### Live Coding
Source code is mounted as volume - edit files on Windows, build in container:

```bash
# Start container with bash
docker-compose run --rm qaos bash

# Inside container - auto-reloads on file changes
cargo watch -x 'build -p os --target x86_64-unknown-none'
```

### Clean Build
```bash
# Remove old build artifacts
docker-compose down -v

# Rebuild from scratch
docker-compose build --no-cache
docker-compose up
```

## Troubleshooting

### QEMU GUI Not Showing
QEMU runs headless by default. To see GUI:
1. Enable X11 forwarding (Linux/WSL2)
2. Or use VNC viewer
3. Or use `-nographic` flag (serial console only)

### Build Cache Issues
```bash
# Clean Cargo cache inside container
docker-compose run --rm qaos cargo clean

# Or rebuild entire image
docker-compose build --no-cache
```

## Why Docker?

✅ **Bypasses LLVM Bug**: Linux toolchain, no Windows alignment issues  
✅ **Reproducible**: Same environment for everyone  
✅ **Portable**: Works on Windows, Linux, macOS  
✅ **CI/CD Ready**: Easy to integrate with GitHub Actions  
✅ **Isolated**: No system pollution, clean environment  

## Next Steps

After successful build:
- User mode process support (requires LLVM bug fix ✅)
- VESA framebuffer (requires LLVM bug fix ✅)
- See ROADMAP.md for full feature list
