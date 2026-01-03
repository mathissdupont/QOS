# QaOS Vision & Roadmap

## 🎯 QaOS Nedir?

**QaOS (Quantum Operating System)**, kuantum hesaplama iş yüklerini birinci sınıf vatandaş olarak ele alan, x86_64 mimarisinde çalışan deneysel bir işletim sistemidir. Geleneksel işletim sistemleri CPU süreçlerini yönetirken, QaOS kuantum devrelerini (circuits) tıpkı normal süreçler gibi yönetir: zamanlama, kaynak tahsisi, izolasyon ve sonuç toplama.

### Temel Amaç

**Kuantum-Klasik Hibrit Hesaplama Ortamı** oluşturmak:
- Klasik CPU kodu ile kuantum devreleri aynı OS üzerinde çalışır
- Kuantum işleri (`jobs`) shell'den veya programlardan submit edilir
- Kernel, işleri zamanlayıcı ile yönetir ve backend'e (simülatör/QPU) gönderir
- Sonuçlar toplanır ve kullanıcıya/programa döndürülür

### Kullanım Senaryoları

1. **Kuantum Algoritma Geliştirme**: QASM2/QASM3 devrelerini doğrudan OS içinden yazıp test etme
2. **Hibrit Hesaplama**: Klasik pre/post-processing + kuantum hesaplama pipeline'ları
3. **Eğitim & Araştırma**: Kuantum hesaplamanın OS seviyesinde nasıl çalıştığını anlama
4. **QPU Entegrasyonu**: Gelecekte gerçek kuantum işlemcilere driver desteği

---

## 🏗️ Mevcut Mimari

```
┌──────────────────────────────────────────────────────────────────┐
│                        USER SPACE                                 │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐   │
│  │   Shell (CMD)   │  │  ELF64 Programs │  │  Web UI (qosd)  │   │
│  │   Commands      │  │  Ring 3 code    │  │  (hosted mode)  │   │
│  └────────┬────────┘  └────────┬────────┘  └────────┬────────┘   │
│           │                    │                    │             │
│           └──────────┬─────────┴────────────────────┘             │
│                      │ Syscall ABI (int 0x80)                     │
├──────────────────────┼───────────────────────────────────────────┤
│                      ▼                                            │
│                 KERNEL SPACE                                      │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │                    QaOS Kernel                               │ │
│  │  ┌────────────┐  ┌────────────┐  ┌────────────────────────┐ │ │
│  │  │ Scheduler  │  │ Job Store  │  │ Quantum Backend        │ │ │
│  │  │ (RR Timer) │  │ (8 slots)  │  │ (Stub Simulator)       │ │ │
│  │  └────────────┘  └────────────┘  └────────────────────────┘ │ │
│  │  ┌────────────┐  ┌────────────┐  ┌────────────────────────┐ │ │
│  │  │ Memory Mgr │  │ VFS Layer  │  │ Device Drivers         │ │ │
│  │  │ (Paging)   │  │ /ram /disk │  │ VGA, KB, ATA, PCI...   │ │ │
│  │  └────────────┘  └────────────┘  └────────────────────────┘ │ │
│  └─────────────────────────────────────────────────────────────┘ │
│                      │                                            │
│                      ▼ Hardware Abstraction                       │
├──────────────────────────────────────────────────────────────────┤
│                     HARDWARE                                      │
│  ┌────────┐  ┌────────┐  ┌────────┐  ┌────────────────────────┐  │
│  │  CPU   │  │  RAM   │  │  Disk  │  │  Future: QPU/FPGA     │  │
│  │ x86_64 │  │        │  │  IDE   │  │  IBM/Google/IonQ...   │  │
│  └────────┘  └────────┘  └────────┘  └────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
```

---

## ✅ Tamamlanan Özellikler

### Kernel Core
- [x] x86_64 bare-metal boot (bootloader crate)
- [x] Paging & heap allocator
- [x] GDT/IDT/TSS setup
- [x] PIT timer (100Hz preemptive scheduling)
- [x] PS/2 Keyboard & Mouse (scroll wheel desteği)
- [x] VGA text mode (80x25, renk desteği, scroll back)
- [x] Serial port (debug output)

### Dosya Sistemi
- [x] RAM-based file system (`/ram`)
- [x] ATA/IDE disk driver (PIO mode)
- [x] Persistent disk file system (`/disk`)
- [x] VFS abstraction layer (`/ram`, `/disk` mount points)
- [x] FAT16 read/write desteği (opsiyonel)

### Process & Scheduling
- [x] Kernel-mode task scheduler (cooperative)
- [x] Ring 3 user mode desteği (şu an devre dışı - LLVM bug)
- [x] Process state tracking (Running, Ready, Exited)
- [x] Foreground/background job control
- [x] Ctrl+C interrupt handling

### Kuantum Subsystem
- [x] Job submission (QASM2 format)
- [x] Job queue & state machine (Queued → Running → Done)
- [x] Deterministic stub simulator backend
- [x] Result collection (measurement counts)
- [x] Job cancellation

### User Interface
- [x] Boot splash screen
- [x] Interactive shell (CMD-like)
- [x] Command history (↑/↓ arrows)
- [x] Text editor (`:w`, `:q`, `:wq`)
- [x] UI overlay toggle

### Hardware Detection
- [x] PCI bus enumeration
- [x] RTC (Real-Time Clock)
- [x] ACPI basics (shutdown, reboot)

---

## ❌ Windows-Benzeri OS İçin Eksikler

### 1. Grafik Arayüz (GUI)
| Özellik | Durum | Öncelik |
|---------|-------|---------|
| Text-mode window manager | 🟡 Temel (overlay) | **Yüksek** |
| Text-mode menu system | 🔴 Yok | **Yüksek** |
| Mouse click handling | 🟡 Temel | **Yüksek** |
| Dialog boxes | 🔴 Yok | Orta |
| Text-mode file explorer | 🔴 Yok | Orta |
| Framebuffer graphics mode | ⏸️ Ertelendi (LLVM) | - |
| Pixel rendering | ⏸️ Ertelendi (LLVM) | - |
| TrueType fonts | ⏸️ Ertelendi (LLVM) | - |
| Desktop icons | ⏸️ Ertelendi (LLVM) | - |

### 2. Dosya Yönetimi
| Özellik | Durum | Öncelik |
|---------|-------|---------|
| Hierarchical directories | 🟡 Temel | Yüksek |
| File permissions (rwx) | 🔴 Yok | Orta |
| File metadata (timestamps) | 🔴 Yok | Orta |
| File explorer GUI | 🔴 Yok | Orta |
| Drag & drop | 🔴 Yok | Düşük |

### 3. Process Management
| Özellik | Durum | Öncelik |
|---------|-------|---------|
| Multi-process (Ring 3) | ⏸️ Ertelendi (LLVM bug) | - |
| Process isolation (page tables) | ✅ Var (per-process CR3) | Yüksek |
| Kernel-mode tasking | ✅ Aktif | Yüksek |
| IPC (pipes, shared memory) | 🔴 Yok | Orta |
| Dynamic linking | 🔴 Yok | Düşük |
| DLL/shared libraries | 🔴 Yok | Düşük |

### 4. Networking
| Özellik | Durum | Öncelik |
|---------|-------|---------|
| NIC driver (e1000/virtio) | 🔴 Yok | Yüksek |
| TCP/IP stack | 🔴 Yok | Yüksek |
| DNS resolver | 🔴 Yok | Orta |
| HTTP client | 🔴 Yok | Orta |
| Web browser | 🔴 Yok | Düşük |

### 5. Audio
| Özellik | Durum | Öncelik |
|---------|-------|---------|
| Sound card driver | 🔴 Yok | Düşük |
| Audio mixer | 🔴 Yok | Düşük |
| System sounds | 🔴 Yok | Düşük |

### 6. Kullanıcı Deneyimi
| Özellik | Durum | Öncelik |
|---------|-------|---------|
| Login/authentication | 🔴 Yok | Orta |
| User accounts | 🔴 Yok | Düşük |
| Settings/Control panel | 🔴 Yok | Düşük |
| Clipboard | 🔴 Yok | Düşük |

---

## 🗺️ Önerilen Roadmap

> ⚠️ **Not**: Windows LLVM'de bir alignment bug var. Assembly gerektiren özellikler 
> (Ring 3 user mode, framebuffer graphics) şimdilik atlanıyor. Text-mode GUI ve 
> pure-Rust özellikler üzerinde ilerliyoruz.

### Phase 1: Stabilizasyon ✅ (Tamamlandı)
1. ✅ Shell "unknown command" bug fix - Her komuta `return` eklendi
2. ✅ Splash screen timing - 3 saniyeye çıkarıldı  
3. ⏸️ Ring 3 user mode - **ATLATILDI** (LLVM asm alignment bug)
4. ✅ Mevcut kernel stabil çalışıyor

### Phase 2: Text-Mode GUI Geliştirme (Mevcut)
> Framebuffer yerine VGA text mode üzerinde zengin GUI
1. ⬜ Gelişmiş pencere sistemi (text-mode windows)
2. ⬜ Menu bar ve dropdown menüler
3. ⬜ Dialog kutuları (confirm, input, file picker)
4. ⬜ Mouse ile tıklanabilir butonlar
5. ⬜ Basit dosya yöneticisi (text-mode file explorer)
6. ⬜ Syntax highlighting (editor için)

### Phase 3: Shell & Kullanıcı Deneyimi
1. ⬜ Tab completion (dosya/komut tamamlama)
2. ⬜ Pipe desteği (`cmd1 | cmd2`)
3. ⬜ Redirection (`>`, `>>`, `<`)
4. ⬜ Environment variables
5. ⬜ Alias tanımlama
6. ⬜ Script dosyaları (.qsh)
7. ⬜ Daha fazla built-in komut (grep, find, wc, sort)

### Phase 4: Dosya Sistemi Geliştirme
1. ⬜ Dizin hiyerarşisi (nested directories)
2. ⬜ File metadata (timestamps, size)
3. ⬜ Symbolic links
4. ⬜ File permissions (basit rwx)
5. ⬜ Mount/unmount komutları
6. ⬜ Disk usage (`du`, `df` komutları)

### Phase 5: Networking (Pure Rust)
1. ⬜ E1000 NIC driver (MMIO, no asm)
2. ⬜ Ethernet frame handling
3. ⬜ ARP protocol
4. ⬜ IPv4 + ICMP (ping)
5. ⬜ UDP sockets
6. ⬜ TCP sockets (basit)
7. ⬜ DHCP client

### Phase 6: Kuantum Subsystem Geliştirme
1. ⬜ Native Rust simülatör (mevcut stub yerine gerçek)
2. ⬜ Daha fazla kuantum gate (T, S, SWAP, Toffoli)
3. ⬜ Multi-qubit desteği artırma (8+ qubit)
4. ⬜ QASM3 parser
5. ⬜ Quantum job priority levels
6. ⬜ Circuit visualization (text-mode)
7. ⬜ Quantum error simulation

### Phase 7: Gelişmiş Özellikler
1. ⬜ Basit task manager (process list, kill)
2. ⬜ System monitor (CPU, memory usage)
3. ⬜ Help sistem (man pages)
4. ⬜ Configuration files
5. ⬜ Boot options menu

### 🚫 Ertelenen Özellikler (LLVM Bug)
> Bu özellikler Windows'ta LLVM alignment bug nedeniyle ertelendi.
> Linux/WSL veya bug düzeltildikten sonra eklenebilir.

| Özellik | Neden Ertelendi |
|---------|-----------------|
| Ring 3 User Mode | `iretq` assembly instruction gerekli |
| Framebuffer Graphics | Bootloader VESA mode assembly sorunlu |
| Custom syscall handler | `syscall`/`sysret` assembly gerekli |
| True preemptive multitasking | Context switch assembly gerekli |

---

## 🔧 Geliştirme

### Build & Run
```powershell
# Interactive QEMU
cargo os-run

# Headless verification
cargo os-verify

# With persistent disk
cargo xtask run-fs

# Hosted web UI (development)
./run-qosd.ps1
```

### Proje Yapısı
```
QOS/
├── crates/
│   ├── qos-abi/          # Syscall ABI definitions
│   ├── qos-core/         # Job scheduler, store (no_std compatible)
│   ├── qos-os-kernel/    # Kernel implementation
│   ├── qosd/             # Hosted daemon (web UI)
│   └── qos-pybridge/     # Python simulator integration
├── docs/
│   ├── SPEC.md           # Technical specification
│   ├── USAGE.md          # User guide
│   └── VISION.md         # This document
└── examples/
    └── bell.qasm         # Example quantum circuit
```

---

## 📚 Referanslar

- [OSDev Wiki](https://wiki.osdev.org/)
- [Writing an OS in Rust](https://os.phil-opp.com/)
- [Qiskit Documentation](https://qiskit.org/documentation/)
- [OpenQASM Specification](https://openqasm.com/)

---

*Son güncelleme: 3 Ocak 2026*
