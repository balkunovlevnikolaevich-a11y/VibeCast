#![windows_subsystem = "windows"]

mod tornado_shell;

mod app;
mod desktop;
mod halloween;
mod light;
mod log;
mod menu;
mod prefs;
mod render;
mod screen;
mod snow;
mod tracker;
mod update;
mod license;
mod welcome;

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows::core::w;

fn test_version() -> bool {
    static TEST: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TEST.get_or_init(|| std::env::args().any(|arg| arg == "--test-version") ||
        std::env::current_exe().ok().and_then(|path| path.file_stem().map(|name| name.to_string_lossy().eq_ignore_ascii_case("VibeCast-Test"))).unwrap_or(false))
}

fn test_capture_dir() -> Option<&'static std::path::Path> {
    static DIR: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| if test_version() {
        std::env::args().find_map(|arg| arg.strip_prefix("--test-capture=").map(std::path::PathBuf::from))
    } else { None }).as_deref()
}

fn main() {
    // What "Uninstall" in Settings runs; it removes everything, even with VibeCast still running.
    if std::env::args().skip(1).any(|argument| argument == "--uninstall") {
        snowhush::install::uninstall();
        return;
    }
    unsafe {
        let _instance = CreateMutexW(None, true, if test_version() { w!("Local\\VibeCastTestVersion") } else { w!("Local\\VibeCast") });
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    snowhush::language::choose();
    log::start();
    std::panic::set_hook(Box::new(|info| log!("panic on the {} thread: {info}", std::thread::current().name().unwrap_or("unnamed"))));
    log!("VibeCast {}", env!("CARGO_PKG_VERSION"));
    if let Err(error) = app::run() {
        log!("failed: {error}");
    }
}
