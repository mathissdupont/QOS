#!/bin/bash
# QOS Desktop GUI Quick Start Script
# This script builds and runs QOS with VNC access

echo "=================================="
echo "  QOS Desktop GUI - VNC Mode"
echo "=================================="
echo ""
echo "🚀 Starting QOS with VNC server..."
echo "📺 VNC will be available at: localhost:5900"
echo ""
echo "To view GUI:"
echo "  1. Open RealVNC Viewer"
echo "  2. Connect to: localhost:5900"
echo "  3. Wait for boot splash"
echo "  4. Type 'desktop' in shell"
echo ""
echo "Press Ctrl+C to stop QEMU"
echo "=================================="
echo ""

cd crates/qos-os-kernel
cargo run
