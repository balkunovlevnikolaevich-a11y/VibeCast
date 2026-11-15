//! Edition-specific GitHub Releases updater. No client credentials are required.
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{PCWSTR,w};

pub struct Release { pub version: String, pub url: String }

pub fn background() {
    use std::os::windows::process::CommandExt;
    let script=include_str!("../../tools/update.ps1");
    let asset=option_env!("VIBECAST_UPDATE_ASSET").unwrap_or("VibeCast-Free-Setup.exe");
    if !["VibeCast-Free-Setup.exe","VibeCast-Pro-Setup.exe"].contains(&asset) {return;}
    let command=format!("& {{ {script} }} -CurrentVersion '{}' -AssetName '{asset}'",env!("CARGO_PKG_VERSION"));
    if let Err(error)=std::process::Command::new("powershell.exe").args(["-NoProfile","-NonInteractive","-WindowStyle","Hidden","-Command",&command]).creation_flags(0x08000000).spawn(){crate::log!("updater start failed: {error}");}
}

pub fn open(url: &str) {
    let url: Vec<u16> = url.encode_utf16().chain([0]).collect();
    unsafe { ShellExecuteW(None,w!("open"),PCWSTR(url.as_ptr()),PCWSTR::null(),PCWSTR::null(),SW_SHOWNORMAL); }
}
