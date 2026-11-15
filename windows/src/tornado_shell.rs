//! Desktop icon movement with a durable, fail-closed recovery journal.
//! All Shell calls are made on the snow thread's COM apartment.
use std::fs;
use std::path::PathBuf;
use windows::Win32::Foundation::POINT;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, FindWindowExW, SendMessageTimeoutW, SMTO_ABORTIFHUNG};
use windows::core::{w, BOOL};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, IServiceProvider, CLSCTX_ALL};
use windows::Win32::System::Variant::{VARIANT, VT_I4};
use windows::Win32::UI::Shell::{
    IFolderView2, IFolderViewOptions, IShellBrowser, IShellWindows, ILGetSize,
    FVO_CUSTOMPOSITION, FWF_AUTOARRANGE, SID_STopLevelBrowser,
    SVGIO_ALLVIEW, SVSI_POSITIONITEM, SWC_DESKTOP, SWFO_NEEDDISPATCH, ShellWindows,
};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN};
use windows::core::Interface;
use snowhush_core::geometry::V2;
use snowhush_core::tornado::{IconState, Tornado};
use snowhush_core::tornado_journal::{self, DesktopItem, DesktopSnapshot};

type Checked<T> = std::result::Result<T, String>;

fn desktop_list() -> Checked<HWND> {
    unsafe extern "system" fn find(window: HWND, data: LPARAM) -> BOOL {
        unsafe {
            if let Ok(view) = FindWindowExW(Some(window), None, w!("SHELLDLL_DefView"), None) {
                if let Ok(list) = FindWindowExW(Some(view), None, w!("SysListView32"), None) {
                    *(data.0 as *mut Option<HWND>) = Some(list);
                    return false.into();
                }
            }
        }
        true.into()
    }
    let mut result = None;
    unsafe { let _ = EnumWindows(Some(find), LPARAM(&raw mut result as isize)); }
    result.ok_or_else(|| "desktop icon view unavailable".into())
}

fn journal_path() -> Checked<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA unavailable")?;
    let dir = PathBuf::from(base).join("VibeCast");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("tornado-recovery.snapshot"))
}

fn folder_view() -> Checked<(IFolderView2, IFolderViewOptions)> {
    unsafe {
        let shell: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let mut location = VARIANT::default();
        (*location.Anonymous.Anonymous).vt = VT_I4;
        (*location.Anonymous.Anonymous).Anonymous.lVal = 0; // CSIDL_DESKTOP
        let empty = VARIANT::default();
        let mut hwnd = 0;
        let dispatch = shell.FindWindowSW(&location, &empty, SWC_DESKTOP, &mut hwnd, SWFO_NEEDDISPATCH).map_err(|e| e.to_string())?;
        let provider: IServiceProvider = dispatch.cast().map_err(|e| format!("desktop service: {e}"))?;
        let browser: IShellBrowser = provider.QueryService(&SID_STopLevelBrowser).map_err(|e| e.to_string())?;
        let view = browser.QueryActiveShellView().map_err(|e| e.to_string())?;
        let folder = view.cast::<IFolderView2>().map_err(|e| e.to_string())?;
        let options = view.cast::<IFolderViewOptions>().map_err(|e| e.to_string())?;
        Ok((folder, options))
    }
}

fn snapshot(folder: &IFolderView2, options: &IFolderViewOptions) -> Checked<DesktopSnapshot> {
    unsafe {
        let count = folder.ItemCount(SVGIO_ALLVIEW).map_err(|e| e.to_string())?;
        if !(0..=10_000).contains(&count) { return Err("unexpected desktop item count".into()); }
        let mut items = Vec::with_capacity(count as usize);
        for index in 0..count {
            let pidl = folder.Item(index).map_err(|e| e.to_string())?;
            if pidl.is_null() { return Err("desktop returned a null item identifier".into()); }
            let len = ILGetSize(Some(pidl)) as usize;
            let result = if (2..=65_536).contains(&len) {
                let id = std::slice::from_raw_parts(pidl.cast::<u8>(), len).to_vec();
                folder.GetItemPosition(pidl).map(|point| DesktopItem { id, x: point.x, y: point.y }).map_err(|e| e.to_string())
            } else { Err("invalid desktop item identifier".into()) };
            CoTaskMemFree(Some(pidl.cast()));
            items.push(result?);
        }
        Ok(DesktopSnapshot { folder_flags: folder.GetCurrentFolderFlags().map_err(|e| e.to_string())?,
            view_options: options.GetFolderViewOptions().map_err(|e| e.to_string())?.0 as u32, items })
    }
}

fn restore_from(path: &PathBuf) -> Checked<()> {
    if !path.exists() { return Ok(()); }
    let saved = tornado_journal::load(path).map_err(|e| format!("recovery file: {e}"))?;
    let (folder, options) = folder_view()?;
    // Match identities, not indices or names; never use a stale PIDL for a deleted item.
    let current = snapshot(&folder, &options)?;
    let present: Vec<&DesktopItem> = saved.items.iter().filter(|old| current.items.iter().any(|item| item.id == old.id)).collect();
    let missing = present.len() != saved.items.len();
    let flag = FWF_AUTOARRANGE.0 as u32;
    let attempt = (|| -> Checked<()> {
        if current.folder_flags & flag != 0 {
            unsafe { folder.SetCurrentFolderFlags(flag, 0) }.map_err(|e| format!("disable auto-arrange for recovery: {e}"))?;
        }
        let changed: Vec<&&DesktopItem> = present.iter().filter(|old| current.items.iter().any(|item|
            item.id == old.id && (item.x != old.x || item.y != old.y))).collect();
        let points: Vec<POINT> = changed.iter().map(|item| POINT { x:item.x, y:item.y }).collect();
        let ids: Vec<*const ITEMIDLIST> = changed.iter().map(|item| item.id.as_ptr().cast()).collect();
        if ids.is_empty() { return Ok(()); }
        unsafe { folder.SelectAndPositionItems(ids.len() as u32, ids.as_ptr(), Some(points.as_ptr()), SVSI_POSITIONITEM.0 as u32) }
            .map_err(|e| format!("restore icon positions: {e}"))
    })();
    // Restore both original flags even when a positioning operation fails.
    let flags_result = if (current.folder_flags | saved.folder_flags) & flag != 0 {
        unsafe { folder.SetCurrentFolderFlags(flag, saved.folder_flags & flag) }.map_err(|e| format!("restore auto-arrange: {e}"))
    } else { Ok(()) };
    attempt?; flags_result?;
    let restored_flags = unsafe { folder.GetCurrentFolderFlags() }.map_err(|e| e.to_string())?;
    let restored_options = unsafe { options.GetFolderViewOptions() }.map_err(|e| e.to_string())?.0 as u32;
    if (restored_flags ^ saved.folder_flags) & flag != 0 ||
        (restored_options ^ saved.view_options) & FVO_CUSTOMPOSITION.0 as u32 != 0 {
        return Err("Windows did not restore desktop view settings; record retained".into());
    }
    for item in &present {
        let point = unsafe { folder.GetItemPosition(item.id.as_ptr().cast()) }.map_err(|e| e.to_string())?;
        if point.x != item.x || point.y != item.y { return Err("Windows rearranged icons during recovery; record retained".into()); }
    }
    if missing { return Err("some desktop items are missing; recovery record retained".into()); }
    fs::remove_file(path).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn recover_pending() -> Checked<()> { restore_from(&journal_path()?) }

/// Read-only probe used before enabling the effect on a given Windows desktop.
pub fn inspect_desktop() -> Checked<(usize, u32, u32)> {
    let (folder, options) = folder_view()?;
    let saved = snapshot(&folder, &options)?;
    // Also validate that the exact identifiers can be durably encoded.
    saved.encode().map_err(|e| e.to_string())?;
    Ok((saved.items.len(), saved.folder_flags, saved.view_options))
}

pub fn inspect_snapshot() -> Checked<DesktopSnapshot> {
    let (folder, options) = folder_view()?;
    snapshot(&folder, &options)
}

pub struct DesktopTornado {
    folder: IFolderView2,
    pub model: Tornado,
    pub origin: POINT,
    saved: DesktopSnapshot,
    path: PathBuf,
    previous: Vec<POINT>,
}

impl DesktopTornado {
    pub fn start() -> Checked<Self> {
        let path = journal_path()?;
        if path.exists() { return Err("previous icon recovery is pending".into()); }
        let (folder, options) = folder_view()?;
        let saved = snapshot(&folder, &options)?;
        let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) } as f32;
        let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) } as f32;
        let positions: Vec<V2> = saved.items.iter().map(|item| V2::new(item.x as f32, item.y as f32)).collect();
        let model = Tornado::new(width, height, &positions).ok_or("invalid desktop dimensions")?;
        // No desktop setting or item is changed until this journal is on disk.
        tornado_journal::save_new(&path, &saved).map_err(|e| format!("cannot save desktop layout: {e}"))?;
        let flag = FWF_AUTOARRANGE.0 as u32;
        let configured = (|| -> Checked<()> {
            if saved.folder_flags & flag != 0 {
                unsafe { folder.SetCurrentFolderFlags(flag, 0) }.map_err(|e| format!("disable auto-arrange: {e}"))?;
            }
            if unsafe { folder.GetCurrentFolderFlags() }.map_err(|e| e.to_string())? & flag != 0 { return Err("Windows did not disable auto-arrange".into()); }
            Ok(())
        })();
        if let Err(error) = configured {
            return match restore_from(&path) { Ok(()) => Err(error), Err(recovery) => Err(format!("{error}; recovery pending: {recovery}")) };
        }
        let previous = saved.items.iter().map(|item| POINT { x:item.x, y:item.y }).collect();
        let origin = POINT { x: unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) }, y: unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) } };
        Ok(Self { folder, model, origin, saved, path, previous })
    }

    pub fn step(&mut self, dt: f32) -> Checked<()> {
        self.model.step(dt);
        let list = desktop_list()?;
        for (i, icon) in self.model.icons.iter().enumerate() {
            if icon.state == IconState::Resting { continue; }
            let point = POINT { x:icon.position.x.round() as i32, y:icon.position.y.round() as i32 };
            if point == self.previous[i] { continue; }
            // Resolve the current index by identity; Explorer can reorder items while the effect runs.
            let count = unsafe { self.folder.ItemCount(SVGIO_ALLVIEW) }.map_err(|e| e.to_string())?;
            let mut index = None;
            for n in 0..count {
                let pidl = unsafe { self.folder.Item(n) }.map_err(|e| e.to_string())?;
                if pidl.is_null() { continue; }
                let len = unsafe { ILGetSize(Some(pidl)) } as usize;
                let same = len == self.saved.items[i].id.len() && unsafe { std::slice::from_raw_parts(pidl.cast::<u8>(), len) } == self.saved.items[i].id;
                unsafe { CoTaskMemFree(Some(pidl.cast())); }
                if same { index = Some(n); break; }
            }
            let index = index.ok_or("desktop item disappeared")?;
            if !(-32768..=32767).contains(&point.x) || !(-32768..=32767).contains(&point.y) { return Err("desktop coordinates exceed ListView limits".into()); }
            let packed = (point.x as u16 as u32) | ((point.y as u16 as u32) << 16);
            let mut result = 0usize;
            let ok = unsafe { SendMessageTimeoutW(list, 0x100f, WPARAM(index as usize), LPARAM(packed as isize), SMTO_ABORTIFHUNG, 1000, Some(&mut result)) };
            if ok.0 == 0 || result == 0 { return Err("Windows rejected icon position".into()); }
            self.previous[i] = point;
        }
        Ok(())
    }

    pub fn restore(&mut self) -> Checked<()> { restore_from(&self.path) }
}

impl Drop for DesktopTornado {
    fn drop(&mut self) { if let Err(error) = self.restore() { crate::log!("tornado recovery pending: {error}"); } }
}
