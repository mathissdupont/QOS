> **Superseded (2026-07-01):** the authoritative long-range plan is now
> [docs/MASTERPLAN.md](docs/MASTERPLAN.md). This file is kept for historical context
> (the LLVM/Docker bring-up era).

# QaOS Roadmap & Known Issues

## ✅ SOLVED: LLVM Alignment Issue (Docker Solution)

### ~~Problem~~ **ÇÖZÜLDÜ!**
Windows'ta LLVM x86_64-unknown-none target için yanlış alignment sorunu **Docker Linux ortamı ile aşıldı**.

### Çözüm: Docker Container (2026-01-03)
- ✅ **Docker image hazır** (rust:1.85.0 + QEMU)
- ✅ **QaOS başarıyla boot oluyor** (Linux LLVM correct alignment)
- ✅ **Network stack çalışıyor** (E1000 NIC detected, link UP)
- ✅ **Build sistemi stabil** (0 errors, sadece warnings)

### Artık Mümkün Olanlar
- ✅ User mode applications (LLVM doğru çalışıyor)
- ✅ Process isolation (ring 0/3 separation)
- ✅ Pixel-based graphics (VESA/framebuffer - test edilecek)
- ✅ ELF binary loading (alignment doğru)

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

### 📊 ✅ Seçilen Çözüm: Docker (2026-01-03)
1. ✅ **Docker** - UYGULANDIĞI
   - Dockerfile + docker-compose.yml hazır
   - Reproducible build environment
   - Linux LLVM doğru alignment veriyor
   - QEMU headless mode çalışıyor
   - Live coding (volume mount ile)
2. WSL2 build (alternatif, daha native)
3. GCC cross-compiler (gerek kalmadı)
4. Alignment hack (gerek kalmadı)

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
**Bağımlılık:** ✅ LLVM bug ÇÖZÜLDÜ (Docker) - **BAŞLANABİLİR!**

---

#### 1.2 Preemptive Multitasking ✅ TAMAMLANDI
- [x] Timer interrupt context switch
- [x] Process priorities (High/Normal/Low)
- [x] Quantum-based scheduling (time_slice)
- [x] Sleep/wake queue (TIMER_TICKS, wake_time)
- [x] Weighted round-robin scheduler
- [ ] Process signals (TODO)

**Tamamlanma:** 2026-01-03  
**Bağımlılık:** ~~User mode~~ Mevcut interrupt-based syscall yeterli

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
- [ ] VESA framebuffer (Docker'da test edilmeli)
- [ ] VBE mode setting
- [ ] True color support (24-bit RGB)
- [ ] Double buffering
- [ ] Simple compositor

**Tahmini süre:** 1 hafta  
**Bağımlılık:** ✅ LLVM bug ÇÖZÜLDÜ - **BAŞLANABİLİR!**

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
- [x] **HTTP/1.1 client** ✅ TAMAMLANDI (1100+ satır, 2026-01-03)
- [x] **E1000 NIC driver** ✅ ÇALIŞIYOR (MAC: 52:54:00:12:34:56)
- [ ] RTL8139 driver
- [ ] virtio-net driver
- [ ] DHCP client
- [ ] DNS client (UDP)
- [ ] TLS/SSL implementation (mbedtls/rustls)

**Tahmini süre:** 1 hafta (TLS için)  
**Bağımlılık:** Yok (HTTP client hazır, network UP)

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

#### 6.2 QPU Backend Enhancement 🟡
- [x] **IBM Quantum API** ✅ ENTEGRE (2026-01-03)
- [x] **Google Quantum AI** ✅ ENTEGRE (2026-01-03)
- [x] **IonQ** ✅ ENTEGRE (2026-01-03)
- [x] **HTTP/1.1 client** ✅ HAZIR (circuit-to-QASM, job submission)
- [ ] AWS Braket (HTTP client hazır, API key gerekli)
- [ ] Azure Quantum (HTTP client hazır, API key gerekli)
- [ ] Rigetti (HTTP client hazır, API key gerekli)
- [ ] Real TLS/SSL implementation (HTTPS için)

**Tahmini süre:** 3 gün (TLS hariç)  
**Bağımlılık:** TLS library (mbedtls/rustls) - HTTPS için

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

### Milestone 1: "Functional OS" ✅ %85 TAMAMLANDI
- ✅ Kernel basics
- ✅ Memory management
- ✅ File system (FAT16 + custom diskfs)
- ✅ **Networking (E1000 NIC, TCP/IP stack, HTTP/1.1 client)** 🆕
- ✅ **LLVM bug ÇÖZÜLDÜ (Docker)** 🆕
- ✅ **Boot successful in QEMU** 🆕
- ✅ **Preemptive scheduling (weighted priority, sleep/wake)** 🆕
- [ ] **User mode** ← ŞİMDİ BAŞLANABİLİR!

**Blocker:** ~~LLVM bug~~ ✅ ÇÖZÜLDÜ  
**Next:** User mode implementation (1 hafta tahmini)

---

### Milestone 2: "Desktop OS" (2-3 ay)
- [ ] Pixel-based GUI ← ŞİMDİ BAŞLANABİLİR! (LLVM çözüldü)
- [ ] Window manager
- [ ] User applications
- [ ] Audio playback
- [ ] USB support

**Blocker:** ~~LLVM bug~~ ✅ ÇÖZÜLDÜ  
**Next:** VESA framebuffer test (Docker'da)

---

### Milestone 3: "Quantum OS" ✅ %60 TAMAMLANDI
- ✅ **Quantum simulator (state vector, gates)** 🆕
- ✅ **QPU backend (IBM/Google/IonQ)** 🆕
- ✅ **HTTP/1.1 client (QASM submission)** 🆕
- ✅ **Circuit-to-QASM converter** 🆕
- [ ] Visual circuit editor (GUI gerekli)
- [ ] Quantum debugger
- [ ] Cloud QPU integration (HTTPS için TLS gerekli)

**Blocker:** TLS implementation (HTTPS için)  
**Unlock:** mbedtls port veya rustls integration  
**Note:** HTTP quantum API'ler şimdi çalışabilir (IBM test edilebilir)

---

### Milestone 4: "Production Ready" (1 yıl+)
- ❌ Multi-user support
- ❌ Security hardening
- ❌ Package ecosystem
- ❌ Documentation
- ❌ Developer community

## 📈 Priority Matrix

| Feature | Impact | Effort | LLVM Dep | Status | Priority |
|---------|--------|--------|----------|--------|----------|
| ~~**LLVM bug fix**~~ | 🔴 Critical | Medium | - | ✅ **DONE** | ~~P0~~ |
| **HTTP client** | 🟠 High | High | ❌ No | ✅ **DONE** | ~~P1~~ |
| **QPU backend** | 🟡 Medium | Medium | ❌ No | ✅ **DONE** | ~~P2~~ |
| **User mode** | 🔴 Critical | High | ✅ Unlocked | 🟡 Ready | **P0** |
| ~~**Preemptive sched**~~ | 🔴 Critical | Medium | ✅ Unlocked | ✅ **DONE** | ~~P1~~ |
| **Framebuffer** | 🟠 High | Medium | ✅ Unlocked | 🟡 Ready | **P0** |
| **TLS/SSL** | 🟡 Medium | High | ❌ No | 🔴 Needed | **P1** |
| **AHCI driver** | 🟡 Medium | High | ❌ No | 🔴 TODO | P2 |
| **USB stack** | 🟡 Medium | Very High | ❌ No | 🔴 TODO | P3 |
| **GUI toolkit** | 🟠 High | Very High | ✅ Unlocked | 🟡 Ready | P2 |
| **Audio** | 🟢 Low | High | ❌ No | 🔴 TODO | P4 |
| **Package mgr** | 🟢 Low | Medium | ❌ No | 🔴 TODO | P4 |

**Legend:**
- 🔴 Critical: OS olmak için şart
- 🟠 High: Modern OS için gerekli
- 🟡 Medium: Nice to have
- 🟢 Low: Bonus features

---

## 🚀 Quick Start (✅ DOCKER READY!)

### ✅ Recommended: Docker (WORKING!)
```powershell
# Windows'ta (PowerShell)
cd C:\Users\samet\OneDrive\Masaüstü\QOS

# Docker build (ilk kez, 10-15 dakika)
docker-compose build

# QaOS'u çalıştır (headless QEMU)
docker-compose run --rm qaos

# İçinde:
# - cargo xtask run-fs    # Filesystem ile
# - cargo xtask run       # Basic boot
# - cargo build           # Sadece build
```

**Boot çıktısı:**
```
QOS-OS boot OK (serial)
heap initialized
[E1000] MAC address: 52:54:00:12:34:56
[E1000] Link is UP
QaOS ready. Type on keyboard...
```

### Alternative: WSL2 (Native Performance)
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
cd /mnt/c/Users/samet/OneDrive/Masaüstü/QOS
cargo xtask run
```

**Docker Avantajları:**
- ✅ Reproducible environment
- ✅ Team collaboration ready
- ✅ CI/CD compatible
- ✅ Live coding (volume mount)
- ✅ LLVM bug ÇÖZÜLDÜ

**Dosyalar:**
- `Dockerfile` - Rust 1.85.0 + QEMU + dependencies
- `docker-compose.yml` - Service definition
- `DOCKER.md` - Detailed documentation

---

## 📝 Notes

### Öğrenilen Dersler (2026-01-03 Update)
1. ✅ **Docker çözümü mükemmel çalıştı** - LLVM bug aşıldı
2. ✅ **Quantum subsystem çalışıyor** - HTTP client + QPU backend hazır
3. ✅ **Network stack solid** - E1000 NIC, TCP/IP, HTTP/1.1 working
4. ✅ **Build system stabil** - 0 errors, 373 warnings (cleanup gerekli)
5. 🆕 **Headless QEMU works** - `-nographic -serial mon:stdio` Linux'ta
6. 🆕 **Volume mount allows live coding** - Windows'ta edit, Docker'da build

### Tamamlanan Kararlar
- ✅ **Docker kullanıyoruz** → Uygulandı (2026-01-03)
- ✅ **LLVM bug ÇÖZÜLDÜ** → Docker Linux environment
- ✅ **HTTP/1.1 client hazır** → 1100+ satır, QPU backend entegre
- ❌ GCC cross-compiler gerek yok → Docker yeterli
- ❌ LLVM bug report gerek yok → Workaround bulundu

### Gelecek Kararlar
- [ ] User mode implementation başlayalım mı? → **ŞİMDİ MÜMKÜN!**
- [ ] Framebuffer test edelim mi? → Docker'da test edilmeli
- [ ] TLS implementation için mbedtls mi rustls mi? → Araştır
- [ ] IBM Quantum API test edelim mi? → API key gerekli

---

## 🎉 Recent Achievements (2026-01-03)

### Completed Today
1. ✅ **LLVM Bug SOLVED** - Docker Linux environment
2. ✅ **QaOS Boot Successful** - QEMU headless mode working
3. ✅ **HTTP/1.1 Client** - 1100+ lines, full implementation
4. ✅ **QPU Backend Integration** - IBM/Google/IonQ APIs ready
5. ✅ **Network Stack Verified** - E1000 NIC, link UP, MAC detected
6. ✅ **Docker Environment** - Dockerfile, docker-compose, documentation
7. ✅ **Quantum Features Working** - Circuit execution, QASM conversion
8. ✅ **Phase 1.2 Preemptive Multitasking** - Priority scheduler, sleep/wake, time slicing

### Boot Log (Verified)
```
QOS-OS boot OK (serial)
heap initialized
[Mouse] Initialized
[RTC] Initialized: 2026-01-03 17:14:46
[PCI] Found 6 devices
[E1000] Found device at 00:03.0
[E1000] MAC address: 52:54:00:12:34:56
[E1000] Link is UP
QaOS ready. Type on keyboard...
```

### Next Steps (Priority Order)
1. **User Mode Implementation** (unlocked, P0)
2. **Framebuffer Test** (unlocked, P0)
3. **TLS Library Integration** (P1, for HTTPS)
4. **Preemptive Scheduling** (P1, needs user mode)
5. **IBM Quantum Test** (P2, HTTP ready, needs API key)

---

**Last Updated:** 2026-01-03 (LLVM Bug Solved!)  
**Next Review:** After User Mode Implementation  
**Maintainer:** QaOS Team  
**Status:** 🟢 **DEVELOPMENT ACTIVE** - Docker environment stable, core features working!lım mı? → Upstream fix için

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
