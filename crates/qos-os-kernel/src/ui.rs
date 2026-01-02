use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use crate::{interrupts, syscall, tasking, vga};
use crate::vga::Color; // Renkleri daha rahat yazmak için

static UI_ENABLED: AtomicBool = AtomicBool::new(false);
static LAST_DRAW_TICKS: AtomicU64 = AtomicU64::new(0);
static LAST_EXIT_SEQ: AtomicU64 = AtomicU64::new(0);

const UI_ROWS: usize = 15;
const COL_SPLIT: usize = 40;

// --- GÖRSEL YARDIMCILAR ---

// Not: `vga::write_at` ASCII dışını 0xfe ile değiştiriyor.
// Bu yüzden çizgiler için sadece ASCII kullanıyoruz.
const CHAR_HORIZ: &str = "-";
const CHAR_VERT: &str = "|";
const CHAR_CROSS: &str = "+";

fn write_fixed(row: usize, col: usize, s: &str, fg: Color, bg: Color, max: usize) {
    if max == 0 { return; }
    let mut buf = alloc::string::String::new();
    if s.chars().count() >= max {
        // Basit dilimleme (UTF-8 karakter sayısına dikkat ederek)
        buf.push_str(&s.chars().take(max).collect::<alloc::string::String>());
    } else {
        buf.push_str(s);
        for _ in 0..(max - s.chars().count()) {
            buf.push(' ');
        }
    }
    vga::write_at(row, col, &buf, fg, bg);
}

// Belirtilen satırı baştan sona belirli bir karakterle çizer
fn draw_hline(row: usize, ch: &str, fg: Color, bg: Color) {
    let mut s = alloc::string::String::new();
    for _ in 0..80 {
        s.push_str(ch);
    }
    vga::write_at(row, 0, &s, fg, bg);
}

// Dikey çizgi çizer (Panel ayırıcı)
fn draw_vline(col: usize, start_row: usize, end_row: usize, ch: &str, fg: Color, bg: Color) {
    for r in start_row..end_row {
        vga::write_at(r, col, ch, fg, bg);
    }
}

pub fn enabled() -> bool {
    UI_ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(on: bool) {
    UI_ENABLED.store(on, Ordering::Relaxed);
    if on {
        vga::set_reserved_top_rows(UI_ROWS);
        vga::clear_screen();
        LAST_DRAW_TICKS.store(0, Ordering::Relaxed);
        LAST_EXIT_SEQ.store(0, Ordering::Relaxed);
    } else {
        vga::set_reserved_top_rows(0);
        vga::clear_screen();
    }
}

pub struct UiTask;

impl UiTask {
    pub fn new() -> Self { Self }

    fn draw(&mut self) {
        // Verileri çek
        let ticks = interrupts::TICKS.load(Ordering::Relaxed);
        let fg_pid = tasking::foreground_pid();
        let running_pid = tasking::current_pid();
        let running_job = syscall::current_running_handle();
        let procs = tasking::list_processes();
        let jobs = syscall::snapshot_jobs();

        // --- 1. HEADER (En Üst) ---
        // Daha az göz yoran, stabil bir palet.
        let header_bg = Color::Black;
        let header_fg = Color::LightGray;

        // clear_row(row, fg, bg)
        vga::clear_row(0, header_fg, header_bg);
        
        let header_text = alloc::format!(
            " QOS | Ticks {:<8} | FG {:<3} | Active P{} J{}",
            ticks, fg_pid, running_pid, running_job
        );
        write_fixed(0, 0, &header_text, header_fg, header_bg, 80);

        // İstatistikleri sağa yaslayalım
        let stats = alloc::format!("Procs: {:<2} Jobs: {:<2} ", procs.len(), jobs.len());
        vga::write_at(0, 80 - stats.len(), &stats, Color::LightCyan, header_bg);

        // --- 2. BAŞLIKLAR VE AYRAÇLAR ---
        // Panel başlıkları için koyu bir şerit.
        let title_bg = Color::Black;
        let title_fg = Color::LightGray;

        vga::clear_row(1, title_fg, title_bg);
        
        // Başlıkları ortalayarak yazalım
        let p_title = " PROCESSES ";
        let j_title = " QUANTUM JOBS ";
        
        // Process başlığı (Sol tarafın ortası)
        let p_start = (COL_SPLIT - p_title.len()) / 2;
        vga::write_at(1, p_start, p_title, Color::LightCyan, title_bg);

        // Job başlığı (Sağ tarafın ortası)
        let j_start = COL_SPLIT + ((80 - COL_SPLIT - j_title.len()) / 2);
        vga::write_at(1, j_start, j_title, Color::LightGreen, title_bg);

        // Dikey Ayraç (Process ve Job arasında)
        draw_vline(COL_SPLIT, 1, UI_ROWS - 1, CHAR_VERT, Color::DarkGray, Color::Black);
        
        // Kesişim noktalarını düzelt (Header ile birleşim)
        // vga::write_at(1, COL_SPLIT, CHAR_VERT, Color::LightGray, title_bg); 

        // --- 3. SÜTUN İSİMLERİ ---
        vga::clear_row(2, Color::LightGray, Color::Black);
        
        // Sol taraf
        let col_header_left = " PID  FG   STATE     EXIT";
        write_fixed(2, 0, col_header_left, Color::DarkGray, Color::Black, COL_SPLIT);
        
        // Sağ taraf (Padding ekleyerek)
        let col_header_right = " SLOT HNDL  ST WL  SHT NQ  IR";
        write_fixed(2, COL_SPLIT + 1, col_header_right, Color::DarkGray, Color::Black, 80 - COL_SPLIT - 1);
        
        // Sütun başlıklarının altına ince bir çizgi
        draw_hline(3, CHAR_HORIZ, Color::DarkGray, Color::Black);
        // Çizginin ortasındaki kesişimi düzelt
        vga::write_at(3, COL_SPLIT, CHAR_CROSS, Color::DarkGray, Color::Black);


        // --- 4. GÖVDE (LİSTELER) ---
        let body_rows = UI_ROWS.saturating_sub(5); // Header(1) + Title(1) + ColHead(1) + Line(1) + Footer(1)
        let start_row = 4;

        // Satırları temizle
        for i in 0..body_rows {
            vga::clear_row(start_row + i, Color::LightGray, Color::Black);
            // Her satıra dikey ayracı tekrar çiz (temizlendiği için)
            vga::write_at(start_row + i, COL_SPLIT, CHAR_VERT, Color::DarkGray, Color::Black);
        }

        // -- Processes (Sol) --
        for (i, (pid, st, code)) in procs.into_iter().take(body_rows).enumerate() {
            let row = start_row + i;
            
            // Minimal vurgu: sadece çalışan satır + FG işareti.
            let (fg_color, bg_color) = if pid == running_pid {
                (Color::White, Color::DarkGray)
            } else {
                (Color::LightGray, Color::Black)
            };

            let fg_mark = if pid == fg_pid { "*" } else { " " };
            // State string'ini biraz temizleyelim (örnek: "Run" vs)
            let st_s = alloc::format!("{:?}", st); 
            
            let line = alloc::format!(" {:<4} {}   {:<8} {}", pid, fg_mark, st_s, code);
            write_fixed(row, 0, &line, fg_color, bg_color, COL_SPLIT);
            
            // Eğer vurgulu satırsa dikey çizginin üzerine taşmaması için boyamayı sınırla
            // (write_fixed zaten sınırları koruyor ama rengi resetlemek gerekebilir)
            vga::write_at(row, COL_SPLIT, CHAR_VERT, Color::DarkGray, Color::Black);

            // Foreground sürecini küçük bir ekstra renkle vurgula (arka planı bozma).
            if pid == fg_pid {
                vga::write_at(row, 6, "*", Color::Yellow, Color::Black);
            }
        }

        // -- Jobs (Sağ) --
        for (i, j) in jobs.into_iter().take(body_rows).enumerate() {
            let row = start_row + i;
            let st_str = syscall::abbrev_state(j.state);
            
            // Duruma göre renk (Örnek: Run=Yeşil, Wait=Sarı)
            let status_color = match st_str {
                "R" | "RUN" => Color::LightGreen,
                "W" | "WAI" => Color::Yellow,
                "D" | "DON" => Color::DarkGray,
                "E" | "ERR" => Color::LightRed,
                _ => Color::LightGray,
            };

            // Progress display: gates done / gates total
            let progress = if j.gates_total > 0 {
                (j.gates_remaining as u32 * 100) / (j.gates_total as u32)
            } else {
                0
            };
            
            // Hizalama: slot handle state q gates%
            let line = alloc::format!(
                " {:<4} {:<5} {:<3} q={} {}%",
                j.slot, j.handle, st_str, j.n_qubits, 100 - progress
            );
            
            // +1 yapıyoruz ki çizginin hemen dibine yazmasın
            write_fixed(row, COL_SPLIT + 1, &line, status_color, Color::Black, 80 - COL_SPLIT - 1);
        }

        // --- 5. FOOTER (Alt Bilgi) ---
        let footer_row = UI_ROWS - 1;
        // Footer için sakin bir şerit
        vga::clear_row(footer_row, Color::DarkGray, Color::Black);
        
        // Komut ipuçları
        let hints = " PgUp/PgDn scroll | ui on/off | exec <path> | killp <pid> | udemo";
        write_fixed(footer_row, 0, hints, Color::LightGray, Color::Black, 80);
    }
}

impl crate::scheduler::Task for UiTask {
    fn step(&mut self) {
        if !enabled() { return; }

        let ticks = interrupts::TICKS.load(Ordering::Relaxed);
        let exit_seq = tasking::exit_seq();
        let last_ticks = LAST_DRAW_TICKS.load(Ordering::Relaxed);
        let last_exit = LAST_EXIT_SEQ.load(Ordering::Relaxed);

        // Her tick'te çizmek yerine, sadece veri değişince veya her N tick'te bir çiz
        // Ekran titremesini azaltmak için.
        if ticks == last_ticks && exit_seq == last_exit && (ticks % 10 != 0) {
            return;
        }

        LAST_DRAW_TICKS.store(ticks, Ordering::Relaxed);
        LAST_EXIT_SEQ.store(exit_seq, Ordering::Relaxed);
        self.draw();
    }
}