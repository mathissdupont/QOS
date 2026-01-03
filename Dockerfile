# QaOS - Quantum Operating System
# Docker build environment (bypasses Windows LLVM alignment bug)

FROM rust:1.85.0

# Install dependencies
RUN apt-get update && apt-get install -y \
    qemu-system-x86 \
    qemu-system-gui \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

# Install Rust nightly and components
RUN rustup toolchain install nightly-2024-12-01 && \
    rustup default nightly-2024-12-01 && \
    rustup component add rust-src llvm-tools-preview && \
    rustup target add x86_64-unknown-none

# Install bootimage
RUN cargo install bootimage

# Set working directory
WORKDIR /qaos

# Copy project files
COPY . .

# Build the OS
RUN cargo build -p os --target x86_64-unknown-none

# Default command: run QaOS in QEMU
CMD ["cargo", "xtask", "run"]
