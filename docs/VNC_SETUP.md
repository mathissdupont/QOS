# QOS Desktop GUI - Görselleştirme Rehberi

## 🖥️ Docker'da GUI Görüntüleme

QOS Desktop GUI'yi Docker içinde çalıştırırken görmek için **VNC** kullanıyoruz.

### Hızlı Başlangıç

#### 1. Docker Container'ı Başlat
```bash
docker-compose up qaos
```

Container çalışmaya başladığında **port 5900** açılacak (VNC sunucusu).

#### 2. VNC Viewer'ı Aç

**RealVNC Viewer** veya başka bir VNC client kullan:

- VNC Adres: `localhost:5900`
- Şifre: Yok (şifresiz)

#### 3. QOS'u Çalıştır

Container içinde:
```bash
cd crates/qos-os-kernel
cargo run
```

#### 4. Desktop'u Başlat

QEMU açılınca shell'de:
```bash
desktop
```

---

## 📋 Alternatif Yöntemler

### Yöntem 1: Tek Komutla Çalıştır

```bash
docker-compose run --rm -p 5900:5900 qaos bash /qaos/run-gui.sh
```

Ardından RealVNC Viewer'da `localhost:5900`'e bağlan.

### Yöntem 2: Manuel Çalıştırma

```bash
# Container'a gir
docker-compose run --rm -p 5900:5900 qaos bash

# İçeride
cd crates/qos-os-kernel
cargo run

# VNC'den bağlan: localhost:5900
```

---

## 🎮 VNC Client Seçenekleri

### Windows'ta:
1. **RealVNC Viewer** (Önerilen)
   - İndir: https://www.realvnc.com/en/connect/download/viewer/
   - Connect to: `localhost:5900`

2. **TigerVNC Viewer**
   - İndir: https://tigervnc.org/
   - Hafif ve hızlı

3. **UltraVNC**
   - İndir: https://uvnc.com/

### Web Tarayıcıdan (noVNC):
Gelecekte eklenecek - web browser'dan direkt erişim

---

## 🐛 Sorun Giderme

### VNC'ye Bağlanamıyorum
```bash
# Port'un açık olduğunu kontrol et
docker ps

# PORTS sütununda şunu görmeli: 0.0.0.0:5900->5900/tcp
```

### Ekran Siyah Görünüyor
- QEMU'nun başlaması 5-10 saniye sürebilir
- Boot splash ekranını bekleyin
- Serial output'a bakın (terminal'de görünür)

### Desktop Komutu Çalışmıyor
```bash
# Shell promptu geldiğinde
help           # Komutları gör
help gui       # GUI yardımı
desktop        # Desktop'u başlat
```

---

## ⚙️ QEMU Parametreleri

Mevcut ayarlar (Cargo.toml):
```bash
qemu-system-x86_64 \
  -drive if=ide,index=0,media=disk,format=raw,file=bootimage.bin \
  -vnc :0 \           # VNC server port 5900
  -serial mon:stdio   # Serial çıktı terminal'de
```

### Özelleştirme

Daha fazla RAM:
```toml
run-command = ["qemu-system-x86_64", "-m", "256M", "-drive", "...", "-vnc", ":0", ...]
```

Farklı VNC port:
```toml
-vnc :1  # Port 5901 kullanır
```

---

## 📊 Performans İpuçları

1. **KVM Desteği** (Linux host'larda):
   ```bash
   # Docker'a KVM erişimi ver
   docker run --device /dev/kvm ...
   ```

2. **Çoklu CPU**:
   ```toml
   run-command = [..., "-smp", "2", ...]  # 2 CPU core
   ```

3. **GPU Acceleration** (gelişmiş):
   ```toml
   -vga virtio
   ```

---

## 🎨 Ekran Çözünürlüğü

Şu an: **80x25 text mode** (VGA)

Gelecekte:
- VESA framebuffer desteği
- Daha yüksek çözünürlükler
- Grafik modu (GUI rendering)

---

## 📸 Ekran Görüntüsü Alma

VNC Viewer'da:
- `Menu → Screenshot` veya `F8`

QEMU'da:
```bash
# Monitor'a geç (Ctrl+Alt+2)
screendump screenshot.ppm
```

---

## 🚀 Hızlı Demo

Tam otomatik demo için:

```bash
# 1. Container başlat (arka planda)
docker-compose up -d qaos

# 2. VNC Viewer aç
# Connect to: localhost:5900

# 3. QOS'u çalıştır
docker exec -it qaos-dev bash -c "cd crates/qos-os-kernel && cargo run"

# 4. Shell'de yazın
desktop
calc
notepad
explorer
```

---

## 📞 Destek

Sorun yaşıyorsanız:
1. Docker logs kontrol et: `docker-compose logs qaos`
2. VNC bağlantısını test et: `telnet localhost 5900`
3. QEMU çıktısını kontrol et (serial output)

---

**QOS Desktop Environment** - VNC ile Windows-tarzı GUI! 🎉
