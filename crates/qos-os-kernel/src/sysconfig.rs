//! System configuration (WP-11): the choices made in first-boot setup — language, user name,
//! theme — persisted as a small `key=value` file.
//!
//! Storage is fallback-first (ADR-0015): the persistent QOSFS data disk (`system.cfg`) when one
//! is attached, otherwise the RAM fs (survives only until reboot; the OOBE warns about this).

extern crate alloc;

use alloc::string::{String, ToString};
use spin::Mutex;

pub const CONFIG_FILE: &[u8] = b"system.cfg";

/// UI language chosen at setup. The wizard itself is fully bilingual; desktop-wide string tables
/// arrive with WP-11 slice 3.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Tr,
}

#[derive(Clone, Debug)]
pub struct SysConfig {
    pub lang: Lang,
    pub user: String,
    pub dark: bool,
    /// True when the config was loaded from / saved to the persistent disk.
    pub on_disk: bool,
}

impl Default for SysConfig {
    fn default() -> Self {
        SysConfig { lang: Lang::En, user: String::new(), dark: true, on_disk: false }
    }
}

static CURRENT: Mutex<Option<SysConfig>> = Mutex::new(None);

/// Serialize as `key=value` lines.
fn to_text(c: &SysConfig) -> String {
    alloc::format!(
        "lang={}\nuser={}\ntheme={}\n",
        if c.lang == Lang::Tr { "tr" } else { "en" },
        c.user,
        if c.dark { "dark" } else { "light" }
    )
}

fn from_text(text: &str, on_disk: bool) -> SysConfig {
    let mut c = SysConfig { on_disk, ..Default::default() };
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        match k.trim() {
            "lang" => c.lang = if v.trim() == "tr" { Lang::Tr } else { Lang::En },
            "user" => c.user = v.trim().to_string(),
            "theme" => c.dark = v.trim() != "light",
            _ => {}
        }
    }
    c
}

/// Load the config from the persistent disk (preferred) or the RAM fs. `None` = first boot
/// (setup should run).
pub fn load() -> Option<SysConfig> {
    if let Some(c) = CURRENT.lock().clone() {
        return Some(c);
    }
    let (bytes, on_disk) = if crate::diskfs::is_formatted() {
        match crate::diskfs::read(CONFIG_FILE) {
            Some(b) => (Some(b), true),
            None => (crate::fs::read(CONFIG_FILE), false),
        }
    } else {
        (crate::fs::read(CONFIG_FILE), false)
    };
    let bytes = bytes?;
    let cfg = from_text(&String::from_utf8_lossy(&bytes), on_disk);
    *CURRENT.lock() = Some(cfg.clone());
    Some(cfg)
}

/// Persist the config: to the QOSFS disk when available (formatting a blank disk on demand),
/// falling back to the RAM fs. Returns the stored config (with `on_disk` reflecting reality).
pub fn save(mut cfg: SysConfig) -> SysConfig {
    let text = to_text(&cfg);
    cfg.on_disk = false;
    if crate::ahci::present() {
        if !crate::diskfs::is_formatted() {
            crate::diskfs::mkfs();
        }
        if crate::diskfs::is_formatted() && crate::diskfs::write(CONFIG_FILE, text.as_bytes()).is_ok() {
            cfg.on_disk = true;
        }
    }
    if !cfg.on_disk {
        let _ = crate::fs::write(CONFIG_FILE, text.as_bytes());
    }
    crate::serial_println!(
        "[SETUP] config saved ({}): lang={:?} user='{}' theme={}",
        if cfg.on_disk { "disk" } else { "ram" },
        cfg.lang,
        cfg.user,
        if cfg.dark { "dark" } else { "light" }
    );
    *CURRENT.lock() = Some(cfg.clone());
    cfg
}

/// The active config (default when setup has not run yet).
pub fn current() -> SysConfig {
    load().unwrap_or_default()
}
