# QaOS - Quantum Operating System
# Docker build environment (bypasses Windows LLVM alignment bug)

FROM rust:1.85.0-bookworm

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

# Copy only cargo files first for better caching
COPY Cargo.toml Cargo.lock ./
COPY crates/qos-os-kernel/Cargo.toml ./crates/qos-os-kernel/
COPY crates/qos-abi/Cargo.toml ./crates/qos-abi/
COPY crates/qos-core/Cargo.toml ./crates/qos-core/
COPY crates/qos-os-xtask/Cargo.toml ./crates/qos-os-xtask/
COPY crates/qos-userdemo/Cargo.toml ./crates/qos-userdemo/
COPY crates/qosd/Cargo.toml ./crates/qosd/
COPY crates/qos-pybridge/Cargo.toml ./crates/qos-pybridge/

# Copy source files
COPY . .

# Default command: run QaOS in QEMU
CMD ["cargo", "xtask", "run"]
