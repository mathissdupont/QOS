# QOS - Quantum Operating System
## VM Kurulum Kılavuzu

**✅ TAM İŞLETİM SİSTEMİ - Çalışır Durumda!**

---

## 📦 Dosyalar

- **ISO Image**: `qos.iso` (22 MB) ⭐ **ÖNERĐLEN - Tüm VM'lerde çalışır**
- **Raw Disk**: `qos-os.bin` (680 KB) - QEMU için alternatif

**ISO kullan** - direkt boot eder, format çevirmeye gerek yok!

---

## 🖥️ VirtualBox ile Çalıştırma (ISO)

### 1. Yeni VM Oluştur

1. VirtualBox'ı aç
2. **Yeni** (New) → **Expert Mode**
3. Ayarlar:
   - **Name**: QOS
   - **Type**: Other
   - **Version**: Other/Unknown (64-bit)
   - **Memory**: 512 MB
   - **Hard Disk**: "Do not add a virtual hard disk" (ISO'dan boot edeceğiz)

### 2. ISO'yu Ekle

1. VM'e sağ tıkla → **Settings**
2. **Storage** → **Controller: IDE** altında CD simgesine tıkla
3. **Choose a disk file** → `qos.iso` seç
4. **OK**

### 3. Network Ayarları (Opsiyonel)

1. **Settings** → **Network** → **Adapter 1**
2. **Attached to**: NAT
3. **Adapter Type**: Intel PRO/1000 MT Desktop (E1000)

### 4. Başlat

**Start** → QOS shell açılacak!

**Test et**:
```bash
submit-bell
jobs          # 5 saniye bekle
result 1      # Bell state: |00⟩ ~50%, |11⟩ ~50%
viz 1         # Histogram görselleştirme
```

---

## 🖥️ VirtualBox ile Çalıştırma (Raw Disk - Alternatif)

**Not**: ISO daha kolay! Sadece raw disk kullanmak istersen:

#### Windows PowerShell ile VDI Dönüşümü:
```powershell
# VirtualBox dizinine git (varsayılan konum)
cd "C:\Program Files\Oracle\VirtualBox"

# Raw image'ı VDI'ye çevir
.\VBoxManage.exe convertfromraw "C:\Users\samet\OneDrive\Masaüstü\QOS\qos-os.bin" "C:\Users\samet\OneDrive\Masaüstü\QOS\qos-os.vdi" --format VDI

# Başarılı olursa:
# "Successfully converted image"
```

Alternatif: Direkt komut satırından:
```powershell
& "C:\Program Files\Oracle\VirtualBox\VBoxManage.exe" convertfromraw "$env:USERPROFILE\OneDrive\Masaüstü\QOS\qos-os.bin" "$env:USERPROFILE\OneDrive\Masaüstü\QOS\qos-os.vdi" --format VDI
```

### 3. VM Ayarları

**System**:
- **Boot Order**: Hard Disk (first)
- **Enable EFI**: ❌ HAYIR (BIOS modunda çalışır)
- **Processor**: 1 CPU yeterli

**Display**:
- **Video Memory**: 16 MB
- **Graphics Controller**: VBoxVGA

**Network**:
- **Adapter 1**: NAT (enable)
- **Adapter Type**: Intel PRO/1000 MT Desktop (E1000)
  - ⚠️ **ÇOK ÖNEMLİ**: E1000 seçilmeli, başka kart çalışmaz!

**Storage**:
- Controller: IDE
- **Attach**: `qos-os.vdi` (yukarıda oluşturduğumuz)

### 4. Çalıştır!

**Start** → VM açılacak ve QOS boot edecek!

---

## 🎮 VMware ile Çalıştırma (ISO)

### 1. Yeni VM Oluştur
1. **Create a New Virtual Machine**
2. **Typical** → Next
3. **Installer disc image file (iso)** → `qos.iso` seç
4. Guest OS: **Other** → **Other 64-bit**

### 2. VM Ayarları
- **Name**: QOS
- **Memory**: 512 MB
- **Network Adapter**: NAT

### 3. Başlat

**Power On** → QOS açılır!

---

## 🎮 VMware ile Çalıştırma (VMDK - Alternatif)

**Not**: ISO daha kolay! Raw disk kullanmak istersen:

```powershell
# QEMU img tool kullan (VMware ile gelen)
& "C:\Program Files (x86)\VMware\VMware Player\OVFTool\qemu-img.exe" convert -f raw -O vmdk "C:\Users\samet\OneDrive\Masaüstü\QOS\qos-os.bin" "C:\Users\samet\OneDrive\Masaüstü\QOS\qos-os.vmdk"
```

Alternatif (QEMU yoksa):
```powershell
# VBoxManage ile VMDK oluştur
& "C:\Program Files\Oracle\VirtualBox\VBoxManage.exe" convertfromraw "$env:USERPROFILE\OneDrive\Masaüstü\QOS\qos-os.bin" "$env:USERPROFILE\OneDrive\Masaüstü\QOS\qos-os.vmdk" --format VMDK
```

---

## 🚀 QEMU ile Çalıştırma (En Kolay!)

QEMU zaten kuruluysa:

```powershell
# Doğrudan çalıştır
qemu-system-x86_64 -drive format=raw,file=qos-os.bin -m 512M -device e1000,netdev=net0 -netdev user,id=net0
```

---

## 🚀 QEMU ile Çalıştırma (EN KOLAY) ⭐

### Windows'ta

**Çift tıkla**:
```
RUN-QOS-ISO.bat
```

veya manuel:
```bash
qemu-system-x86_64 -cdrom qos.iso -m 512M -device e1000,netdev=net0 -netdev user,id=net0
```

### Linux/Mac

```bash
qemu-system-x86_64 -cdrom qos.iso -m 512M -device e1000,netdev=net0 -netdev user,id=net0
```

**Kısayollar**:
- `Ctrl+Alt+2`: QEMU monitor
- `Ctrl+Alt+F`: Fullscreen
- `Ctrl+Alt+G`: Fare yakala/bırak

---

## 🎯 İşletim Sistemi Özellikleri

### ✅ Çalışan Sistemler

**Kernel**:
- x86_64 bare metal kernel
- VGA text mode (80x25)
- Memory management (heap, paging)
- Interrupt handling (timer, keyboard, mouse)

**Donanım**:
- ✅ PS/2 Keyboard
- ✅ PS/2 Mouse (scroll wheel destekli)
- ✅ Intel E1000 Network Card
- ✅ Real-time Clock (RTC)
- ✅ PCI Bus scanning
- ✅ Serial port (debug)

**Network Stack**:
- ✅ Ethernet frames
- ✅ ARP protocol
- ✅ IPv4 packets
- ✅ UDP sockets
- ✅ TCP sockets (basic)
- ✅ DHCP client
- ⚠️ ICMP/Ping (partial)

**Dosya Sistemi**:
- ✅ VFS (Virtual File System)
- ✅ In-memory FS
- ✅ Basic operations (read, write, mkdir, rm)

**Quantum Computing** (UNIQUE!):
- ✅ Real statevector simulator (up to 32 qubits)
- ✅ OpenQASM 2.0 parser
- ✅ Bell state circuits
- ✅ Job scheduler (16 concurrent jobs)
- ✅ ASCII visualization
- ✅ Gates: H, X, Y, Z, CX, CZ, SWAP, Toffoli, Rx, Ry, Rz

**Shell**:
- ✅ Command line interface
- ✅ Tab completion
- ✅ Command history
- ✅ Pipes and redirects
- ✅ Environment variables
- ✅ 60+ built-in commands

### ⚠️ Bilinen Sınırlamalar

- **User mode disabled** (LLVM asm bug)
- **No framebuffer** (bootloader 0.9.x limitation - VGA text only)
- **No ACPI** (power management limited)
- **No disk persistence** (in-memory only)

---

## 📝 Test Komutları

VM başladıktan sonra shell'de dene:

```bash
# Sistem Bilgisi
uname                # Kernel versiyonu
uptime               # Sistem çalışma süresi
meminfo              # Bellek kullanımı
qubits               # Quantum kaynakları

# Network
ifconfig             # Network durumu
netstat              # Bağlantılar
dhcp                 # DHCP ile IP al

# Dosya Sistemi
ls                   # Dosyaları listele
touch test.txt       # Dosya oluştur
cat test.txt         # Dosya oku
mkdir mydir          # Dizin oluştur

# Quantum Computing (UNIQUE!)
submit-bell          # Bell circuit gönder
jobs                 # Job durumunu gör
result 1             # Sonuçları al
viz 1                # Görselleştir

# Beklenen: |00⟩ ~50% ve |11⟩ ~50% (Bell state entanglement!)
```

---

## 🎓 Akademik Değer

Bu işletim sistemi:

1. **Bare metal x86_64 kernel** ✅
2. **Network stack from scratch** ✅
3. **Real quantum simulator** ✅ (Benzersiz!)
4. **Device drivers** ✅ (E1000, PS/2, RTC)
5. **Multi-tasking** ✅ (cooperative scheduler)
6. **Shell environment** ✅ (60+ commands)

**Toplam Satır**: ~15,000+ lines of Rust code

---

## 🐛 Sorun Giderme

### VM açılmıyor
- BIOS mode'da olduğundan emin ol (EFI değil)
- Boot order'da Hard Disk ilk sırada olmalı

### Network çalışmıyor
- E1000 adapter seçili olmalı
- NAT modunda olmalı

### Ekran boş
- VGA text mode destekli VM kullan
- VirtualBox: VBoxVGA seçili olmalı

### Bootloader hatası
- Disk image bozulmuş olabilir
- Tekrar Docker'dan kopyala

---

## 📚 Daha Fazla Bilgi

- **Kaynak Kod**: `C:\Users\samet\OneDrive\Masaüstü\QOS\`
- **Dokümantasyon**: `docs/` klasörü
- **Test Kılavuzu**: `MANUAL_TESTS.md`
- **Sistem Durumu**: `SYSTEM_STATUS.md`

---

**🎉 Tebrikler! Kendi Quantum İşletim Sisteminiz Hazır!**
