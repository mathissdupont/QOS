use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use crate::{interrupts, syscall, tasking, vga};
use crate::vga::Color;

static UI_ENABLED: AtomicBool = AtomicBool::new(false);
static LAST_DRAW_TICKS: AtomicU64 = AtomicU64::new(0);
static LAST_EXIT_SEQ: AtomicU64 = AtomicU64::new(0);

const UI_ROWS: usize = 15;
const COL_SPLIT: usize = 40;

// ASCII-only characters for VGA compatibility
const CHAR_HORIZ: &str = "-";
const CHAR_VERT: &str = "|";
const CHAR_CROSS: &str = "+";

/// Kernel version string
const KERNEL_VERSION: &str = "QaOS v0.1.0";

fn write_fixed(row: usize, col: usize, s: &str, fg: Color, bg: Color, max: usize) {
    if max == 0 { return; }
    let mut buf = alloc::string::String::new();
    if s.chars().count() >= max {
        buf.push_str(&s.chars().take(max).collect::<alloc::string::String>());
    } else {
        buf.push_str(s);
        for _ in 0..(max - s.chars().count()) {
            buf.push(' ');
        }
    }
    vga::write_at(row, col, &buf, fg, bg);
}

fn draw_hline(row: usize, ch: &str, fg: Color, bg: Color) {
    let mut s = alloc::string::String::new();
    for _ in 0..80 {
        s.push_str(ch);
    }
    vga::write_at(row, 0, &s, fg, bg);
}

fn draw_vline(col: usize, start_row: usize, end_row: usize, ch: &str, fg: Color, bg: Color) {
    for r in start_row..end_row {
        vga::write_at(r, col, ch, fg, bg);
    }
}

/// Format uptime from ticks (100Hz)
fn format_uptime(ticks: u64) -> alloc::string::String {
    let total_secs = ticks / 100;
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    alloc::format!("{}:{:02}:{:02}", hours, mins, secs)
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
        let ticks = interrupts::TICKS.load(Ordering::Relaxed);
        let fg_pid = tasking::foreground_pid();
        let running_pid = tasking::current_pid();
        let running_job = syscall::current_running_handle();
        let procs = tasking::list_processes();
        let jobs = syscall::snapshot_jobs();

        // --- 1. TITLE BAR (Top) ---
        let header_bg = Color::Blue;
        let header_fg = Color::White;

        vga::clear_row(0, header_fg, header_bg);
        
        // Left: Kernel version
        vga::write_at(0, 1, KERNEL_VERSION, header_fg, header_bg);
        
        // Center: Uptime and ticks
        let uptime = format_uptime(ticks);
        let center_text = alloc::format!("Up: {} | Ticks: {}", uptime, ticks);
        let center_col = (80 - center_text.len()) / 2;
        vga::write_at(0, center_col, &center_text, Color::LightCyan, header_bg);
        
        // Right: Process/Job count
        let stats = alloc::format!("P:{} J:{} ", procs.len(), jobs.len());
        vga::write_at(0, 80 - stats.len(), &stats, Color::Yellow, header_bg);

        // --- 2. PANEL HEADERS ---
        let title_bg = Color::Black;

        vga::clear_row(1, Color::LightGray, title_bg);
        
        let p_title = " PROCESSES ";
        let j_title = " QUANTUM JOBS ";
        
        // Process title (left panel center)
        let p_start = (COL_SPLIT - p_title.len()) / 2;
        vga::write_at(1, p_start, p_title, Color::LightCyan, title_bg);

        // Job title (right panel center)
        let j_start = COL_SPLIT + ((80 - COL_SPLIT - j_title.len()) / 2);
        vga::write_at(1, j_start, j_title, Color::LightGreen, title_bg);

        // Vertical separator
        draw_vline(COL_SPLIT, 1, UI_ROWS - 1, CHAR_VERT, Color::DarkGray, Color::Black);

        // --- 3. COLUMN HEADERS ---
        vga::clear_row(2, Color::LightGray, Color::Black);
        
        // Left: Processes
        let col_header_left = " PID  FG   STATE     EXIT";
        write_fixed(2, 0, col_header_left, Color::DarkGray, Color::Black, COL_SPLIT);
        
        // Right: Jobs
        let col_header_right = " SLOT HNDL  ST  Q  PROG";
        write_fixed(2, COL_SPLIT + 1, col_header_right, Color::DarkGray, Color::Black, 80 - COL_SPLIT - 1);
        
        // Horizontal separator
        draw_hline(3, CHAR_HORIZ, Color::DarkGray, Color::Black);
        vga::write_at(3, COL_SPLIT, CHAR_CROSS, Color::DarkGray, Color::Black);

        // --- 4. BODY (Lists) ---
        let body_rows = UI_ROWS.saturating_sub(5);
        let start_row = 4;

        // Clear body rows
        for i in 0..body_rows {
            vga::clear_row(start_row + i, Color::LightGray, Color::Black);
            vga::write_at(start_row + i, COL_SPLIT, CHAR_VERT, Color::DarkGray, Color::Black);
        }

        // -- Processes (Left Panel) --
        for (i, (pid, st, code)) in procs.into_iter().take(body_rows).enumerate() {
            let row = start_row + i;
            
            // Highlight running process
            let (fg_color, bg_color) = if pid == running_pid {
                (Color::White, Color::DarkGray)
            } else {
                (Color::LightGray, Color::Black)
            };

            let fg_mark = if pid == fg_pid { "*" } else { " " };
            let st_s = alloc::format!("{:?}", st); 
            
            let line = alloc::format!(" {:<4} {}   {:<8} {}", pid, fg_mark, st_s, code);
            write_fixed(row, 0, &line, fg_color, bg_color, COL_SPLIT);
            
            vga::write_at(row, COL_SPLIT, CHAR_VERT, Color::DarkGray, Color::Black);

            // Highlight foreground marker
            if pid == fg_pid {
                vga::write_at(row, 6, "*", Color::Yellow, Color::Black);
            }
        }

        // -- Jobs (Right Panel) --
        for (i, j) in jobs.into_iter().take(body_rows).enumerate() {
            let row = start_row + i;
            let st_str = syscall::abbrev_state(j.state);
            
            // Color based on state
            let status_color = match st_str {
                "R" | "RUN" => Color::LightGreen,
                "W" | "WAI" => Color::Yellow,
                "D" | "DON" => Color::DarkGray,
                "E" | "ERR" => Color::LightRed,
                _ => Color::LightGray,
            };

            // Progress calculation
            let progress = if j.gates_total > 0 {
                100 - ((j.gates_remaining as u32 * 100) / (j.gates_total as u32))
            } else {
                100
            };
            
            let line = alloc::format!(
                " {:<4} {:<5} {:<3} {} {:>3}%",
                j.slot, j.handle, st_str, j.n_qubits, progress
            );
            
            write_fixed(row, COL_SPLIT + 1, &line, status_color, Color::Black, 80 - COL_SPLIT - 1);
        }

        // --- 5. FOOTER (Help hints) ---
        let footer_row = UI_ROWS - 1;
        vga::clear_row(footer_row, Color::Black, Color::DarkGray);
        
        // Help hints
        let hints = " Ctrl+C:kill fg | ui on/off | help | PgUp/Dn:scroll | F1:? ";
        write_fixed(footer_row, 0, hints, Color::White, Color::DarkGray, 80);
        
        // Show active process/job
        if running_pid != 0 || running_job != 0 {
            let active = alloc::format!("FG:{} J:{}", fg_pid, running_job);
            vga::write_at(footer_row, 80 - active.len() - 1, &active, Color::Yellow, Color::DarkGray);
        }
    }
}

impl crate::scheduler::Task for UiTask {
    fn step(&mut self) {
        if !enabled() { return; }

        let ticks = interrupts::TICKS.load(Ordering::Relaxed);
        let exit_seq = tasking::exit_seq();
        let last_ticks = LAST_DRAW_TICKS.load(Ordering::Relaxed);
        let last_exit = LAST_EXIT_SEQ.load(Ordering::Relaxed);

        // Throttle drawing to reduce flicker
        if ticks == last_ticks && exit_seq == last_exit && (ticks % 10 != 0) {
            return;
        }

        LAST_DRAW_TICKS.store(ticks, Ordering::Relaxed);
        LAST_EXIT_SEQ.store(exit_seq, Ordering::Relaxed);
        self.draw();
    }
}