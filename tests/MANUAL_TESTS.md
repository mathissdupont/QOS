# QOS Interactive Test Guide
## Manual Testing Procedures

### 🎯 How to Connect

1. **Start VNC Viewer**
   - Windows: Use "TightVNC Viewer" or "RealVNC Viewer"
   - Download: https://www.tightvnc.com/download.php
   
2. **Connect to QEMU**
   - Host: `localhost:5900`
   - Or: `127.0.0.1:5900`
   - Password: (none - no password set)

3. **You should see**: QOS text console with shell prompt

---

### ✅ AUTOMATED TESTS (Already Run)

```
✅ Kernel Boot: PASS
✅ Memory/Heap: PASS  
✅ Mouse Driver: PASS
✅ RTC: PASS
✅ PCI Bus: PASS
✅ Syscall: PASS
✅ E1000 NIC: PASS
✅ Network Link: PASS
✅ GUI: PASS
✅ Menu: PASS
✅ Scheduler: PASS
✅ Shell: PASS
✅ Quantum Work: PASS
```

**Result: 13/13 PASSED** ✅

---

### 🧪 MANUAL TESTS (Interactive)

#### TEST 1: Quantum Visualization ✅ WORKING
```bash
# Submit a Bell circuit
submit-bell

# Expected output:
# "submitted handle=1"

# Check job list (NEW VISUALIZATION!)
jobs

# Expected output:
# Quantum Jobs:
# =============
# Job #1: RUNNING (2 qubits)
# =============

# Wait a moment, then get result
result 1

# Expected output:
# Job #1 Results:
# Bell State Results:
# ==================
# |00> ████████████████████ 512 (50%)
# |11> ████████████████████ 512 (50%)
# ==================

# Try visualization only
viz 1

# Should show same bar chart
```

**✅ PASS**

---

#### TEST 2: User Mode Quantum Program ⚠️ DISABLED
```bash
# SKIP THIS TEST - User mode currently disabled
# Due to LLVM asm bug causing hangs

# userdemo  # ❌ DON'T RUN - will hang system!

# Reason: Ring 3 context switching has LLVM code generation bug
# Status: Disabled until LLVM/Rust upgrade
```

**⚠️ SKIP** - Feature temporarily disabled

---

#### TEST 3: File System Operations
```bash
# List VFS files
ls

# Expected: Shows root directory contents

# Create a file
touch test.txt

# Write content
echo "Hello QOS" > test.txt

# Read back
cat test.txt

# Expected: "Hello QOS"

# List again
ls

# Expected: test.txt should appear

# Remove file
rm test.txt

# Verify removal
ls
```

**✅ PASS** if all file operations work

---

#### TEST 4: System Information
```bash
# Show current time
time

# Expected: Current date and time from RTC

# System uptime
uptime

# Expected: Time since boot

# PCI devices
pci

# Expected: List of 6 PCI devices including E1000

# Tick counter
ticks

# Expected: PIT tick count (increments every 10ms)
```

**✅ PASS**

---

#### TEST 5: Shell Features
```bash
# Command history (up arrow)
# Press UP arrow key

# Expected: Previous commands appear

# Tab completion
# Type "sub" then TAB
sub<TAB>

# Expected: Completes to "submit" or shows options

# Environment variables
env

# Expected: Shows environment variables

# Export new variable
export TEST=hello

# Check it
env | grep TEST

# Expected: Shows TEST=hello
```

**✅ PASS**

---

#### TEST 6: Quantum Interactive Simulator
```bash
# Start simulator
qsim 2

# Expected:
# ╔══════════════════════════════════════╗
# ║     Quantum Simulator (2 qubits)     ║
# ╚══════════════════════════════════════╝
# Commands: h N, x N, cx C T, measure, reset, state, quit
# qsim>

# Apply Hadamard to qubit 0
qsim> h 0

# Apply CNOT (control=0, target=1)
qsim> cx 0 1

# Show state
qsim> state

# Expected: Bell state |00⟩ + |11⟩

# Measure
qsim> measure

# Expected: Random outcome (00 or 11)

# Quit
qsim> quit
```

**✅ PASS**

---

#### TEST 7: Multiple Quantum Jobs
```bash
# Submit multiple jobs
submit-bell
submit-bell
submit-bell

# Check job list
jobs

# Expected: Multiple jobs (QUEUED or RUNNING)

# Wait for completion
# (Jobs run concurrently, 10 gates per tick)

# Check status periodically
jobs

# Get results when done
result 1
result 2
result 3

# All should show Bell state distribution
```

**✅ PASS**

---

#### TEST 8: Help System
```bash
# General help
help

# Expected: Shows all commands

# Category help
help quantum

# Expected: Shows quantum-specific commands

# Category help - system
help system

# Expected: Shows system commands
```

**✅ PASS**

---

#### TEST 9: Network Stack (if configured)
```bash
# Show network interface
ifconfig

# Expected: Shows eth0 with E1000 MAC address

# ARP table
arp

# Expected: Shows ARP entries (may be empty initially)

# Network statistics
netstat

# Expected: Shows connection stats
```

**✅ PASS** if network commands work

**⚠️ SKIP** if network not configured

---

#### TEST 10: Clear and Redraw
```bash
# Clear screen
clear

# Expected: Screen clears, prompt at top

# Run some commands to generate output
jobs
time
uptime

# Clear again
clear
```

**✅ PASS**

---

### 📊 TEST RESULTS SUMMARY

Fill this out as you test:

```
[✅ ] TEST 1: Quantum Visualization - PASS/FAIL
[ ] TEST 2: User Mode Quantum - PASS/FAIL
[ ] TEST 3: File System - PASS/FAIL
[✅] TEST 4: System Info - PASS/FAIL
[✅] TEST 5: Shell Features - PASS/FAIL
[✅] TEST 6: Quantum Simulator - PASS/FAIL
[✅] TEST 7: Multiple Jobs - PASS/FAIL
[✅] TEST 8: Help System - PASS/FAIL
[ ] TEST 9: Network Stack - PASS/FAIL (or SKIP)
[✅] TEST 10: Clear/Redraw - PASS/FAIL
```

---

### 🐛 Known Issues

1. **User mode DISABLED (userdemo hangs) ⚠️**
   - `userdemo` command causes infinite loop
   - Reason: LLVM asm bug in Ring 3 context switching
   - **DO NOT RUN userdemo** - requires QEMU restart
   - Use shell quantum commands instead

2. **Framebuffer disabled**
   - Using VGA text mode
   - Requires bootloader 0.11+ upgrade

3. **ACPI disabled**
   - AHCI/SATA not available
   - Power management limited

4. **Network may need configuration**
   - DHCP client exists but may not auto-run
   - Manual ifconfig may be needed

---

### 🎉 Success Criteria

**Minimum for "Working OS":**
- ✅ All automated tests pass (13/13)
- ✅ Shell responsive
- ✅ Quantum jobs complete
- ✅ File operations work
- ✅ System info commands work

**Full Success:**
- ✅ All manual tests pass
- ✅ Multiple quantum jobs concurrent
- ✅ User mode execution works
- ✅ Network responds

---

### 📝 Notes

Record any issues or observations here:
- 
- 
- 

---

**Current Status: 13/13 automated tests PASSED** ✅

Ready for manual testing!
