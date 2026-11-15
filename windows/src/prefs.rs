use std::ffi::c_void;
use std::time::{SystemTime, UNIX_EPOCH};

use windows::Win32::Foundation::{ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_DWORD, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::Win32::UI::WindowsAndMessaging::{SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW};
use windows::core::{BOOL, PCWSTR, w};

use snowhush_core::halloween::Choice;
use snowhush_core::modes::{self, Picked};
use snowhush_core::simulation::Mode;

fn key() -> PCWSTR { if crate::test_version() { w!("Software\\VibeCast\\TestVersion") } else { w!("Software\\VibeCast") } }
const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("VibeCast");
const STARTUP_APPROVED: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run");

#[derive(Clone, Copy)]
pub struct Settings {
    /// How much falls, in percent: 0 to 100 in steps of 5.
    pub intensity: u32,
    /// Snow and leaves settle along the bottom of the screen.
    pub ground: bool,
    pub melt: bool,
    pub snow_on_windows: bool,
    pub behind_windows: bool,
    /// The snow stays over full-screen apps instead of fading out of their way.
    pub over_full_screen: bool,
    /// The snow settles at the bottom of the screen, under the taskbar, for a taskbar made see-through, as by TranslucentTB.
    pub behind_taskbar: bool,
    pub show_in_screenshots: bool,
    /// The snow also settles on what the desktop draws without a window of its own: a selection dragged across it.
    pub deep_interaction: bool,
    pub welcomed: bool,
    /// Unix seconds of the first run, or 0 for an install older than the stamp.
    pub installed_at: u32,
    pub halloween: Choice,
    /// What falls: the kind picked last, or two mixed.
    pub picked: Picked,
    /// Drawn at the display's highest rate all the time, instead of only while something is pushed around.
    pub max_performance: bool,
    /// How opaque what falls is drawn, in percent: 10 to 100 in steps of 5.
    pub opacity: u32,
    /// Small machine that sweeps snow along the bottom edge, 0 tractor to 3 penguin.
    pub winter_cleaner: u32,
    /// Seconds between completed passes; zero keeps the machine running continuously.
    pub winter_interval: u32,
    pub winter_garlands: bool,
    /// Wind from left to right, in the inclusive range -1 to 1.
    pub winter_wind: i32,
    /// 0 off, 1 cabin, 2 porch.
    pub winter_scene: u32,
    pub winter_paused: bool,
    /// Snowballs collide with windows.
    pub winter_window_walls: bool,
    /// Seconds a snowball mark remains on a window; zero disables marks.
    pub winter_splat_seconds: u32,
    pub rain_thunder: bool,
    pub rain_person: bool,
    pub rain_water: bool,
    /// Session-only: never start an icon-moving effect automatically at login.
    pub tornado_active: bool,
}

impl Settings {
    /// Whether snow settles anywhere.
    pub fn accumulates(&self) -> bool {
        self.ground || self.snow_on_windows
    }

    pub fn load() -> Settings {
        let calm = !animations_enabled();
        // Before the slider, the intensity was one of Off, Light, Steady, Heavy and Variable.
        let stop = read(w!("snowfall")).map(|i| i.min(4) as usize);
        let welcomed = read(w!("welcomed")).is_some_and(|v| v != 0);
        let installed_at = read(w!("installedAt")).unwrap_or_else(|| {
            let stamp = if welcomed { 0 } else { now() };
            write(w!("installedAt"), stamp);
            stamp
        });
        // Before Accumulate picked where snow settles, it turned settling off everywhere at once.
        let settled = read(w!("accumulate")).is_none_or(|v| v != 0) || read(w!("settleGround")).is_some();
        Settings {
            intensity: read(w!("intensity")).map(|v| v.clamp(0, 100)).or(stop.map(|i| [0, 25, 50, 75, 50][i])).unwrap_or(if calm { 25 } else { 50 }),
            ground: read(w!("settleGround")).map_or(settled, |v| v != 0),
            melt: read(w!("meltSnow")).is_none_or(|v| v != 0),
            snow_on_windows: settled && read(w!("snowOnWindows")).is_none_or(|v| v != 0),
            behind_windows: read(w!("behindWindows")).is_some_and(|v| v != 0),
            over_full_screen: read(w!("showOverFullScreen")).is_some_and(|v| v != 0),
            behind_taskbar: read(w!("behindTaskbar")).is_some_and(|v| v != 0),
            show_in_screenshots: read(w!("showInScreenshots")).is_some_and(|v| v != 0),
            deep_interaction: read(w!("deepInteraction")).is_some_and(|v| v != 0),
            welcomed,
            installed_at,
            halloween: Choice { on: read(w!("halloweenOn")).is_some_and(|v| v != 0), skipped: read(w!("halloweenSkipped")).unwrap_or(0) },
            max_performance: read(w!("performance")).is_some_and(|v| v == 1),
            opacity: read(w!("fallingOpacity")).map_or(100, |v| v.clamp(10, 100)),
            winter_cleaner: if read(w!("winterCleaner")) == Some(6) { 6 } else { 0 },
            winter_interval: read(w!("winterInterval")).unwrap_or(60).min(3600),
            winter_garlands: read(w!("winterGarlands")).is_some_and(|v| v != 0),
            winter_wind: (read(w!("winterWind")).unwrap_or(100).min(200) as i32 - 100).clamp(-100, 100),
            // The simplified menu no longer exposes these features; start with both disabled.
            winter_scene: 0,
            winter_paused: false,
            winter_window_walls: read(w!("winterWindowWalls")).is_none_or(|v| v != 0),
            winter_splat_seconds: read(w!("winterSplatSeconds")).unwrap_or(10).min(60),
            rain_thunder: read(w!("rainThunder")).is_some_and(|v| v != 0),
            rain_person: read(w!("rainPerson")).is_none_or(|v| v != 0),
            rain_water: read(w!("rainWater")).is_none_or(|v| v != 0),
            tornado_active: false,
            picked: Picked::load(read(w!("mode")), read(w!("autumn")), Some(0), [read(w!("mix0")), read(w!("mix1"))]),
        }
    }

    pub fn save(&self) {
        write(w!("intensity"), self.intensity);
        write(w!("settleGround"), self.ground as u32);
        write(w!("meltSnow"), self.melt as u32);
        write(w!("snowOnWindows"), self.snow_on_windows as u32);
        write(w!("behindWindows"), self.behind_windows as u32);
        write(w!("showOverFullScreen"), self.over_full_screen as u32);
        write(w!("behindTaskbar"), self.behind_taskbar as u32);
        write(w!("showInScreenshots"), self.show_in_screenshots as u32);
        write(w!("deepInteraction"), self.deep_interaction as u32);
        write(w!("welcomed"), self.welcomed as u32);
        write(w!("installedAt"), self.installed_at);
        write(w!("halloweenOn"), self.halloween.on as u32);
        write(w!("halloweenSkipped"), self.halloween.skipped);
        write(w!("mode"), modes::number(self.picked.mode));
        write(w!("mixed"), 0);
        write(w!("mix0"), modes::number(self.picked.mix[0]));
        write(w!("mix1"), modes::number(self.picked.mix[1]));
        write(w!("autumn"), (self.picked.mode == Mode::Autumn) as u32);
        write(w!("performance"), self.max_performance as u32);
        write(w!("fallingOpacity"), self.opacity);
        write(w!("winterCleaner"), if self.winter_cleaner == 6 { 6 } else { 0 });
        write(w!("winterInterval"), self.winter_interval.min(3600));
        write(w!("winterGarlands"), self.winter_garlands as u32);
        write(w!("winterWind"), (self.winter_wind.clamp(-100, 100) + 100) as u32);
        write(w!("winterScene"), self.winter_scene.min(2));
        write(w!("winterPaused"), self.winter_paused as u32);
        write(w!("winterWindowWalls"), self.winter_window_walls as u32);
        write(w!("winterSplatSeconds"), self.winter_splat_seconds.min(60));
        write(w!("rainThunder"), self.rain_thunder as u32);
        write(w!("rainPerson"), self.rain_person as u32);
        write(w!("rainWater"), self.rain_water as u32);
    }
}

const WINTER_IMAGE: PCWSTR = w!("winterImage");

pub fn winter_image() -> Option<String> {
    let mut buffer = vec![0u16; 32768];
    let mut size = (buffer.len() * 2) as u32;
    let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, key(), WINTER_IMAGE, RRF_RT_REG_SZ, None, Some(buffer.as_mut_ptr().cast::<c_void>()), Some(&mut size)) };
    if status != ERROR_SUCCESS { return None; }
    let end = buffer.iter().position(|&unit| unit == 0).unwrap_or(0);
    (end > 0).then(|| String::from_utf16_lossy(&buffer[..end]))
}

pub fn set_winter_image(path: Option<&str>) {
    unsafe {
        if let Some(path) = path {
            let value: Vec<u16> = path.encode_utf16().chain([0]).collect();
            let _ = RegSetKeyValueW(HKEY_CURRENT_USER, key(), WINTER_IMAGE, REG_SZ.0, Some(value.as_ptr().cast()), (value.len() * 2) as u32);
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, key(), WINTER_IMAGE);
        }
    }
}

pub fn opens_at_login() -> bool {
    let mut buffer = [0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;
    let status =
        unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN, RUN_VALUE, RRF_RT_REG_SZ, None, Some(buffer.as_mut_ptr().cast::<c_void>()), Some(&mut size)) };
    if status != ERROR_SUCCESS {
        return false;
    }
    let length = buffer.iter().position(|&c| c == 0).unwrap_or(0);
    command().is_some_and(|command| command[..command.len() - 1] == buffer[..length]) && !startup_disabled()
}

/// Whether the system's startup list has VibeCast switched off: its record's first byte is odd then.
fn startup_disabled() -> bool {
    let mut record = [0u8; 64];
    let mut size = record.len() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            STARTUP_APPROVED,
            RUN_VALUE,
            RRF_RT_REG_BINARY,
            None,
            Some(record.as_mut_ptr().cast::<c_void>()),
            Some(&mut size),
        )
    };
    status == ERROR_SUCCESS && size > 0 && record[0] & 1 == 1
}

pub fn set_opens_at_login(on: bool) -> WIN32_ERROR {
    if crate::test_version() { return ERROR_SUCCESS; }
    unsafe {
        if !on {
            return RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, RUN_VALUE);
        }
        let Some(command) = command() else { return WIN32_ERROR(1) };
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, STARTUP_APPROVED, RUN_VALUE);
        RegSetKeyValueW(HKEY_CURRENT_USER, RUN, RUN_VALUE, REG_SZ.0, Some(command.as_ptr().cast()), (command.len() * 2) as u32)
    }
}

/// The quoted path of this executable, NUL-terminated.
fn command() -> Option<Vec<u16>> {
    let path = std::env::current_exe().ok()?;
    Some(format!("\"{}\"", path.display()).encode_utf16().chain([0]).collect())
}

fn read(name: PCWSTR) -> Option<u32> {
    let mut value = 0u32;
    let mut size = 4u32;
    let status =
        unsafe { RegGetValueW(HKEY_CURRENT_USER, key(), name, RRF_RT_REG_DWORD, None, Some((&raw mut value).cast::<c_void>()), Some(&mut size)) };
    (status == ERROR_SUCCESS).then_some(value)
}

fn now() -> u32 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as u32)
}

fn write(name: PCWSTR, value: u32) {
    unsafe {
        let _ = RegSetKeyValueW(HKEY_CURRENT_USER, key(), name, REG_DWORD.0, Some((&raw const value).cast::<c_void>()), 4);
    }
}

/// The "Animation effects" setting: off means the person prefers less motion.
pub fn animations_enabled() -> bool {
    let mut enabled = BOOL(1);
    let result = unsafe {
        SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some((&raw mut enabled).cast::<c_void>()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0))
    };
    result.is_err() || enabled.as_bool()
}
