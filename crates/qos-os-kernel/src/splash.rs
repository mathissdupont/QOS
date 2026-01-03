//! Boot splash screen for QOS
//!
//! Displays a centered ASCII splash screen before entering the shell.
//! In verify mode, the splash is shown briefly without delay to maintain determinism.

use crate::{arch, interrupts, keyboard, vga};
use core::sync::atomic::Ordering;

/// Show the boot splash screen.
/// - In verify mode: display briefly, no delay
/// - In interactive mode: wait ~1 second or until first keypress
pub fn show_splash() {
    // Clear screen and display the splash
    vga::clear_screen();
    
    // Center the splash on an 80x25 VGA screen
    // "QaOS" art is about 50 chars wide, centered
    let start_row = 6;
    
    // ASCII art for "QaOS" - must be ASCII-only for VGA compatibility
    draw_centered_text(start_row, r"   ____        ___   ____  ");
    draw_centered_text(start_row + 1, r"  / __ \      /   | / __ \ ");
    draw_centered_text(start_row + 2, r" | |  | | __ / /| || |  | |");
    draw_centered_text(start_row + 3, r" | |  | |/ _` / ___ || |  | |");
    draw_centered_text(start_row + 4, r" | |__| | (_| \__/ || |__| |");
    draw_centered_text(start_row + 5, r"  \___\_\\__,_|   |_| \____/ ");
    draw_centered_text(start_row + 6, r"                 __       ");
    draw_centered_text(start_row + 7, r"                / ___|     ");
    draw_centered_text(start_row + 8, r"                \___ \     ");
    draw_centered_text(start_row + 9, r"                 ___) |    ");
    draw_centered_text(start_row + 10, r"                |____/     ");
    
    // Title and author
    draw_centered_colored(start_row + 12, "Quantum Operating System", vga::Color::LightCyan, vga::Color::Black);
    draw_centered_colored(start_row + 14, "made by mathissdupont", vga::Color::LightGray, vga::Color::Black);
    
    // Version info
    draw_centered_colored(start_row + 16, "v0.1.0", vga::Color::DarkGray, vga::Color::Black);
    
    // Show init progress
    #[cfg(not(feature = "verify"))]
    {
        draw_centered_colored(20, "Initializing...", vga::Color::Yellow, vga::Color::Black);
    }
}

/// Show progress during boot initialization
pub fn show_progress(msg: &str) {
    #[cfg(not(feature = "verify"))]
    {
        // Clear the progress line and show new message
        vga::clear_row(20, vga::Color::LightGray, vga::Color::Black);
        draw_centered_colored(20, msg, vga::Color::Yellow, vga::Color::Black);
    }
    
    // In verify mode, skip visual progress to maintain determinism
    #[cfg(feature = "verify")]
    {
        let _ = msg; // suppress unused warning
    }
}

/// Wait for keypress or timeout before proceeding
/// In verify mode, returns immediately to maintain determinism
pub fn wait_for_continue() {
    #[cfg(feature = "verify")]
    {
        // In verify mode, skip the wait entirely for deterministic behavior
        return;
    }
    
    #[cfg(not(feature = "verify"))]
    {
        // Show "Press any key" message
        draw_centered_colored(22, "Press any key to continue...", vga::Color::White, vga::Color::Black);
        
        // Wait for ~1 second (100 ticks at 100Hz) or until keypress
        let start_ticks = interrupts::TICKS.load(Ordering::Relaxed);
        let timeout_ticks = 100000; // 1 second at 100Hz PIT
        
        loop {
            // Check for keypress
            if keyboard::pop_scancode().is_some() {
                // Drain any remaining scancodes from the press
                while keyboard::pop_scancode().is_some() {}
                break;
            }
            
            // Check for timeout
            let current_ticks = interrupts::TICKS.load(Ordering::Relaxed);
            if current_ticks.saturating_sub(start_ticks) >= timeout_ticks {
                break;
            }
            
            // Yield CPU
            arch::hlt();
        }
    }
}

/// Draw text centered on screen
fn draw_centered_text(row: usize, text: &str) {
    let col = center_column(text.len());
    vga::write_at(row, col, text, vga::Color::LightGreen, vga::Color::Black);
}

/// Draw colored text centered on screen
fn draw_centered_colored(row: usize, text: &str, fg: vga::Color, bg: vga::Color) {
    let col = center_column(text.len());
    vga::write_at(row, col, text, fg, bg);
}

/// Calculate the starting column to center text of given length
fn center_column(text_len: usize) -> usize {
    const SCREEN_WIDTH: usize = 80;
    if text_len >= SCREEN_WIDTH {
        0
    } else {
        (SCREEN_WIDTH - text_len) / 2
    }
}
