//! Quantum Visualization Module
//! 
//! ASCII-based visualization for quantum measurement results

use crate::{vga, syscall};
use alloc::string::{String, ToString};

/// Draw an ASCII bar chart for quantum measurement results
pub fn draw_histogram(results: &[u64], n_qubits: usize, max_width: usize) {
    if results.is_empty() {
        vga::println!("No results to display");
        return;
    }
    
    let num_states = 1 << n_qubits; // 2^n_qubits
    
    // Find max value for scaling
    let max_count = results.iter().max().copied().unwrap_or(1);
    
    vga::println!("Quantum Measurement Results ({}-qubit):", n_qubits);
    vga::println!("========================================");
    
    for (state, &count) in results.iter().enumerate().take(num_states) {
        if count == 0 {
            continue; // Skip zero counts
        }
        
        // Format state as binary
        let state_str = format_binary_state(state, n_qubits);
        
        // Calculate bar width
        let bar_width = if max_count > 0 {
            ((count as usize * max_width) / max_count as usize).max(1)
        } else {
            0
        };
        
        // Draw bar
        let bar = "█".repeat(bar_width);
        vga::println!("|{}> {} {}", state_str, bar, count);
    }
    
    vga::println!("========================================");
}

/// Format a state number as binary string
fn format_binary_state(state: usize, n_qubits: usize) -> String {
    let mut s = String::new();
    for i in (0..n_qubits).rev() {
        if (state >> i) & 1 == 1 {
            s.push('1');
        } else {
            s.push('0');
        }
    }
    s
}

/// Draw a simple Bell state result visualization
pub fn draw_bell_result(res0: u64, res1: u64) {
    let total = res0 + res1;
    if total == 0 {
        vga::println!("No measurements");
        return;
    }
    
    vga::println!("Bell State Results:");
    vga::println!("==================");
    
    // Calculate percentages
    let pct0 = (res0 * 100) / total;
    let pct1 = (res1 * 100) / total;
    
    // Draw |00> bar
    let bar0 = ((res0 as usize * 40) / total as usize).max(1);
    let bar_str0 = "█".repeat(bar0);
    vga::println!("|00> {} {} ({}%)", bar_str0, res0, pct0);
    
    // Draw |11> bar
    let bar1 = ((res1 as usize * 40) / total as usize).max(1);
    let bar_str1 = "█".repeat(bar1);
    vga::println!("|11> {} {} ({}%)", bar_str1, res1, pct1);
    
    vga::println!("==================");
}

/// Show all quantum jobs
pub fn list_jobs() {
    use crate::syscall;
    
    vga::println!("Quantum Jobs:");
    vga::println!("=============");
    
    let mut found_any = false;
    
    // Access job slots
    for i in 0..syscall::max_jobs() {
        if let Some(job_info) = syscall::get_job_info(i) {
            let state_str = match job_info.state {
                syscall::JobState::Queued => "QUEUED",
                syscall::JobState::Running => "RUNNING",
                syscall::JobState::Done => "DONE",
                syscall::JobState::Cancelled => "CANCELLED",
                syscall::JobState::Failed => "FAILED",
            };
            vga::println!("Job #{}: {} ({} qubits)", 
                job_info.handle, state_str, job_info.n_qubits);
            found_any = true;
        }
    }
    
    if !found_any {
        vga::println!("  (no active jobs)");
    }
    
    vga::println!("=============");
}
