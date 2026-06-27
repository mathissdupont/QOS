#!/bin/bash
# Automated quantum test with keyboard input simulation

echo "=== Automated Quantum Test ==="

# Kill existing QEMU
pkill qemu
sleep 2

# Clear serial log
rm -f /tmp/qemu-serial.log

# Start QEMU with -monitor for control
echo "Starting QEMU..."
qemu-system-x86_64 \
  -drive format=raw,file=/workspace/target/x86_64-unknown-none/release/bootimage-os.bin \
  -serial file:/tmp/qemu-serial.log \
  -vga std \
  -m 512M \
  -device e1000,netdev=net0 \
  -netdev user,id=net0 \
  -vnc :0 \
  -monitor unix:/tmp/qemu-monitor.sock,server,nowait \
  -nographic &

QEMU_PID=$!
echo "QEMU PID: $QEMU_PID"

# Wait for boot
echo "Waiting for boot..."
sleep 10

# Send keyboard commands via monitor
echo "Sending submit-bell command..."
echo "sendkey s" | socat - UNIX-CONNECT:/tmp/qemu-monitor.sock
echo "sendkey u" | socat - UNIX-CONNECT:/tmp/qemu-monitor.sock  
echo "sendkey b" | socat - UNIX-CONNECT:/tmp/qemu-monitor.sock
echo "sendkey m" | socat - UNIX-CONNECT:/tmp/qemu-monitor.sock
echo "sendkey i" | socat - UNIX-CONNECT:/tmp/qemu-monitor.sock
echo "sendkey t" | socat - UNIX-CONNECT:/tmp/qemu-monitor.sock
echo "sendkey minus" | socat - UNIX-CONNECT:/tmp/qemu-monitor.sock
echo "sendkey b" | socat - UNIX-CONNECT:/tmp/qemu-monitor.sock
echo "sendkey e" | socat - UNIX-CONNECT:/tmp/qemu-monitor.sock
echo "sendkey l" | socat - UNIX-CONNECT:/tmp/qemu-monitor.sock
echo "sendkey l" | socat - UNIX-CONNECT:/tmp/qemu-monitor.sock
echo "sendkey ret" | socat - UNIX-CONNECT:/tmp/qemu-monitor.sock

# Wait for execution
sleep 3

# Check logs
echo ""
echo "=== Quantum Debug Logs ==="
grep "\[QUANTUM\]" /tmp/qemu-serial.log | tail -30

echo ""
echo "=== All Logs (last 50) ==="
tail -50 /tmp/qemu-serial.log

# Cleanup
kill $QEMU_PID
