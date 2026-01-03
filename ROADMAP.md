# QaOS Roadmap & Known Issues

## 🐛 CRITICAL BUG: LLVM Alignment Issue (Windows)

### Problem
Windows'ta LLVM x86_64-unknown-none target için **yanlış alignment** üretiyor:
- User mode processes crash ediyor (ring 3 geçişinde)
- VESA framebuffer erişimi hatalı
- Context switch sırasında alignment fault

### Etkilenen Özellikler
- ❌ User mode applications
- ❌ Process isolation (ring 0/3 separation)
- ❌ Pixel-based graphics (VESA/framebuffer)
- ❌ ELF binary loading (çalışıyor ama crash)

### Çözüm Seçenekleri

#### ✅ **Seçenek 1: Linux/WSL Build (ÖNERİLEN)**
```bash
# WSL2 içinde
sudo apt install build-essential qemu-system-x86
rustup target add x86_64-unknown-none
cargo xtask run
```

**Avantajlar:**
- LLVM doğru çalışır
- User mode + framebuffer işler
- Native QEMU performansı

**Dezavantajlar:**
- WSL kurulumu gerekli
- Development Windows'tan WSL'e taşınmalı

---

#### 🔧 **Seçenek 2: GCC Cross-Compiler**
```bash
# x86_64-elf-gcc kullan
rustup component add rust-src
cargo rustc -- -C linker=x86_64-elf-gcc
```

**Avantajlar:**
- Windows'ta kalabilirsin
- LLVM bypass edilir

**Dezavantajlar:**
- GCC cross-compiler kurulumu
- Rust + GCC linking sorunları
- Maintenance yükü

---

#### 🛠️ **Seçenek 3: Alignment Workaround (Hack)**
```rust
// Tüm struct'lara explicit alignment
#[repr(C, align(16))]
struct ProcessContext { ... }

// Assembly'de manuel align
.align 16
```

**Avantajlar:**
- Kod değişikliği ile olası fix
- Windows'ta kalabilirsin

**Dezavantajlar:**
- Her struct'ı düzeltmek gerekir
- Garanti değil
- Hacky çözüm

---

#### 🔄 **Seçenek 4: Docker Container**
```dockerfile
FROM rust:latest
RUN rustup target add x86_64-unknown-none
VOLUME /workspace
CMD ["cargo", "xtask", "run"]
```

**Avantajlar:**
- İzole Linux ortamı
- Takım için portable

**Dezavantajlar:**
- Docker overhead
- QEMU GUI Windows'ta göstermek zor

---

### 📊 Öncelik Sırası
1. **WSL2 build** (1 saat setup, kalıcı çözüm)
2. GCC cross-compiler (2-3 saat, karmaşık)
3. Docker (orta yol)
4. Alignment hack (son çare)

---

## 📋 Feature Roadmap

### Phase 1: Temel OS Stabilizasyonu (LLVM fix sonrası)

#### 1.1 User Mode (Kritik) 🔴
- [ ] Ring 3 process creation
- [ ] Syscall interface (SYSCALL/SYSRET)
- [ ] User stack allocation
- [ ] ELF loader düzeltme
- [ ] Context switch asm (user ↔ kernel)

**Tahmini süre:** 1 hafta  
**Bağımlılık:** LLVM bug fix

---

#### 1.2 Preemptive Multitasking 🔴
- [ ] Timer interrupt context switch
- [ ] Process priorities
- [ ] Quantum-based scheduling
- [ ] Sleep/wake queue
- [ ] Process signals

**Tahmini süre:** 4-5 gün  
**Bağımlılık:** User mode

---

#### 1.3 Memory Protection 🔴
- [ ] Per-process page tables
- [ ] Copy-on-write (fork)
- [ ] Demand paging
- [ ] Swap file support
- [ ] Memory permissions enforce

**Tahmini süre:** 1 hafta  
**Bağımlılık:** User mode

---

### Phase 2: Device Driver Genişletme

#### 2.1 Storage Drivers 🟡
- [ ] AHCI driver (SATA)
- [ ] NVMe driver (modern SSD)
- [ ] USB mass storage
- [ ] Partition table (GPT)
- [ ] ext2/ext4 read support

**Tahmini süre:** 2 hafta  
**Bağımlılık:** Yok (başlanabilir)

---

#### 2.2 Graphics 🟠
- [ ] VESA framebuffer (LLVM fix gerekli)
- [ ] VBE mode setting
- [ ] True color support (24-bit RGB)
- [ ] Double buffering
- [ ] Simple compositor

**Tahmini süre:** 1 hafta  
**Bağımlılık:** LLVM bug fix

---

#### 2.3 Input Drivers 🟢
- [ ] USB HID (keyboard/mouse)
- [ ] Multi-touch support
- [ ] Input event queue
- [ ] Keyboard layouts (US, TR)

**Tahmini süre:** 5 gün  
**Bağımlılık:** USB stack

---

#### 2.4 Network Expansion 🟢
- [ ] RTL8139 driver
- [ ] virtio-net driver
- [ ] DHCP client
- [ ] DNS client (UDP)
- [ ] HTTP/1.1 client improvements (TLS)

**Tahmini süre:** 1 hafta  
**Bağımlılık:** Yok (paralel çalışılabilir)

---

#### 2.5 Audio 🟡
- [ ] AC97 driver
- [ ] Intel HDA driver
- [ ] PCM output
- [ ] Simple mixer
- [ ] WAV playback

**Tahmini süre:** 1-2 hafta  
**Bağımlılık:** Yok

---

### Phase 3: System Services

#### 3.1 Init System 🔴
- [ ] Init process (PID 1)
- [ ] Service manager
- [ ] Runlevels/targets
- [ ] Dependency resolution
- [ ] Service restart policies

**Tahmini süre:** 1 hafta  
**Bağımlılık:** User mode

---

#### 3.2 IPC Mechanisms 🟡
- [ ] Unix domain sockets
- [ ] Shared memory (mmap)
- [ ] Message queues
- [ ] Semaphores
- [ ] Signal system (POSIX)

**Tahmini süre:** 1 hafta  
**Bağımlılık:** User mode

---

#### 3.3 Logging & Monitoring 🟢
- [ ] Kernel log buffer (dmesg)
- [ ] Syslog daemon
- [ ] Process monitoring (top/ps)
- [ ] Disk usage (df)
- [ ] Network stats (netstat)

**Tahmini süre:** 3-4 gün  
**Bağımlılık:** File system

---

### Phase 4: User Environment

#### 4.1 Shell Improvements 🟢
- [x] Tab completion (var)
- [x] Pipes (var)
- [ ] Job control (bg/fg)
- [ ] Redirection (>, <, >>)
- [ ] Shell scripting (basic)
- [ ] Environment variables expansion

**Tahmini süre:** 4 gün  
**Bağımlılık:** Signals, job control

---

#### 4.2 Standard Utilities 🟢
- [ ] File ops: cp, mv, rm, mkdir, rmdir
- [ ] Text: cat, more, less, grep, sed
- [ ] Archive: tar, gzip
- [ ] Network: ping, wget, curl
- [ ] System: ps, top, kill, uptime

**Tahmini süre:** 1 hafta  
**Bağımlılık:** User mode

---

#### 4.3 GUI Framework 🟠
- [ ] Window manager (pixel-based)
- [ ] Widget toolkit (buttons, textbox)
- [ ] Font rendering (TrueType)
- [ ] Image loading (PNG, JPEG)
- [ ] Desktop environment

**Tahmini süre:** 3-4 hafta  
**Bağımlılık:** Framebuffer (LLVM fix)

---

### Phase 5: Advanced Features

#### 5.1 Security & Users 🟡
- [ ] User accounts (/etc/passwd)
- [ ] Password hashing
- [ ] Login system
- [ ] File permissions enforcement
- [ ] sudo implementation
- [ ] SELinux-like MAC

**Tahmini süre:** 1-2 hafta  
**Bağımlılık:** User mode

---

#### 5.2 POSIX Compliance 🟡
- [ ] POSIX syscalls (fork, exec, wait)
- [ ] Standard C library (libc)
- [ ] Dynamic linking (ld.so)
- [ ] POSIX threads (pthreads)
- [ ] POSIX signals

**Tahmini süre:** 2-3 hafta  
**Bağımlılık:** User mode, IPC

---

#### 5.3 Package Management 🟢
- [ ] Package format (.qaos)
- [ ] Dependency resolver
- [ ] Repository support
- [ ] Install/remove/update
- [ ] Binary cache

**Tahmini süre:** 1 hafta  
**Bağımlılık:** File system

---

#### 5.4 Virtualization 🟡
- [ ] KVM support
- [ ] VM management
- [ ] virtio drivers (guest)
- [ ] Container support (cgroups-like)

**Tahmini süre:** 2-3 hafta  
**Bağımlılık:** Advanced memory mgmt

---

### Phase 6: Quantum OS Features (Unique!)

#### 6.1 Quantum Simulator Improvements 🟢
- [x] Basic simulator (var)
- [ ] Sparse state vector
- [ ] GPU acceleration (CUDA/ROCm)
- [ ] Noise models
- [ ] Error mitigation
- [ ] Circuit optimization

**Tahmini süre:** 2 hafta  
**Bağımlılık:** GPU driver

---

#### 6.2 QPU Backend Enhancement 🟡
- [x] IBM Quantum API (var)
- [x] Google Quantum AI (var)
- [x] IonQ (var)
- [ ] AWS Braket
- [ ] Azure Quantum
- [ ] Rigetti
- [ ] Real TLS/SSL implementation

**Tahmini süre:** 1 hafta  
**Bağımlılık:** TLS library (mbedtls/rustls)

---

#### 6.3 Quantum Development Tools 🟢
- [ ] QASM editor (syntax highlight)
- [ ] Circuit visualizer (ASCII/graphics)
- [ ] State inspector
- [ ] Quantum debugger
- [ ] Benchmarking suite

**Tahmini süre:** 1-2 hafta  
**Bağımlılık:** GUI framework

---

#### 6.4 Quantum Applications 🟢
- [ ] Shor's algorithm demo
- [ ] Grover's search
- [ ] VQE (chemistry)
- [ ] QAOA (optimization)
- [ ] Quantum ML demos

**Tahmini süre:** Ongoing  
**Bağımlılık:** Quantum subsystem

---

## 🎯 Milestone Targets

### Milestone 1: "Functional OS" (3-4 hafta)
- ✅ Kernel basics
- ✅ Memory management
- ✅ File system
- ✅ Networking
- ❌ **User mode** ← LLVM bug blocker
- ❌ **Preemptive scheduling** ← depends on user mode

**Blocker:** LLVM bug  
**Unlock:** WSL build veya GCC cross-compiler

---

### Milestone 2: "Desktop OS" (2-3 ay)
- ❌ Pixel-based GUI ← LLVM bug blocker
- ❌ Window manager
- ❌ User applications
- ❌ Audio playback
- ❌ USB support

**Blocker:** LLVM bug (framebuffer)  
**Unlock:** WSL build

---

### Milestone 3: "Quantum OS" (4-6 ay)
- ✅ Quantum simulator
- ✅ QPU backend
- ❌ Visual circuit editor
- ❌ Quantum debugger
- ❌ Cloud QPU integration (TLS)

**Blocker:** TLS implementation  
**Unlock:** mbedtls port veya rustls integration

---

### Milestone 4: "Production Ready" (1 yıl+)
- ❌ Multi-user support
- ❌ Security hardening
- ❌ Package ecosystem
- ❌ Documentation
- ❌ Developer community

---

## 📈 Priority Matrix

| Feature | Impact | Effort | LLVM Dep | Priority |
|---------|--------|--------|----------|----------|
| **User mode** | 🔴 Critical | High | ✅ Yes | P0 |
| **LLVM bug fix** | 🔴 Critical | Medium | - | **P0** |
| **Preemptive sched** | 🔴 Critical | Medium | ✅ Yes | P1 |
| **Framebuffer** | 🟠 High | Medium | ✅ Yes | P1 |
| **AHCI driver** | 🟡 Medium | High | ❌ No | P2 |
| **USB stack** | 🟡 Medium | Very High | ❌ No | P3 |
| **Audio** | 🟢 Low | High | ❌ No | P4 |
| **TLS/SSL** | 🟡 Medium | High | ❌ No | P2 |
| **GUI toolkit** | 🟠 High | Very High | ✅ Yes | P2 |
| **Package mgr** | 🟢 Low | Medium | ❌ No | P4 |

**Legend:**
- 🔴 Critical: OS olmak için şart
- 🟠 High: Modern OS için gerekli
- 🟡 Medium: Nice to have
- 🟢 Low: Bonus features

---

## 🚀 Quick Start (LLVM Fix)

### Option 1: WSL2 (Recommended)
```powershell
# Windows'ta WSL2 kur
wsl --install -d Ubuntu

# WSL içinde
sudo apt update
sudo apt install build-essential qemu-system-x86 git curl
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
rustup target add x86_64-unknown-none

# QaOS build
git clone /mnt/c/Users/samet/OneDrive/Masaüstü/QOS ~/qaos
cd ~/qaos
cargo xtask run
```

### Option 2: Docker
```dockerfile
# Dockerfile.qaos
FROM rust:1.75
RUN rustup target add x86_64-unknown-none
RUN apt-get update && apt-get install -y qemu-system-x86
WORKDIR /workspace
CMD ["cargo", "xtask", "run"]
```

```powershell
# Build & run
docker build -t qaos-build -f Dockerfile.qaos .
docker run -v ${PWD}:/workspace -it qaos-build
```

---

## 📝 Notes

### Öğrenilen Dersler
1. **Windows LLVM bug'ı ciddi** - Production için Linux şart
2. **Quantum subsystem çalışıyor** - Unique feature başarılı
3. **Network stack solid** - TCP/IP implementation güçlü
4. **Text-mode GUI yeterli** - Pixel mode bonus

### Gelecek Kararlar
- [ ] WSL2'ye mi geçelim? → **ÖNERİLEN**
- [ ] GCC cross-compiler deneyelim mi? → Fallback
- [ ] LLVM bug report açalım mı? → Upstream fix için

### Community Feedback Needed
- User mode ne kadar kritik? (Şu an tüm kod kernel'da)
- GUI pixel-based olmalı mı? (Text mode yeterli olabilir)
- Hangi QPU provider öncelik? (IBM/Google/IonQ)

---

## 🎓 Referanslar

### Benzer Projeler
- **Redox OS** (Rust): https://www.redox-os.org/
- **SerenityOS** (C++): https://serenityos.org/
- **ToaruOS** (C): https://toaruos.org/
- **IncludeOS** (C++): https://www.includeos.org/

### OS Dev Resources
- OSDev Wiki: https://wiki.osdev.org/
- Philipp Oppermann's Blog: https://os.phil-opp.com/
- Rust OSDev: https://github.com/rust-osdev

### Quantum Computing
- Qiskit (IBM): https://qiskit.org/
- Cirq (Google): https://quantumai.google/cirq
- PennyLane: https://pennylane.ai/

---

**Last Updated:** 2026-01-03  
**Next Review:** After LLVM bug fix  
**Maintainer:** QaOS Team
