//! Diagnostic for actual Shell movement and crash recovery.
#[path = "../tornado_shell.rs"] mod tornado_shell;
#[macro_export] macro_rules! log { ($($arg:tt)*) => { eprintln!($($arg)*) }; }

fn run() -> Result<(), String> {
    use std::{process::Command, thread, time::Duration};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok() }.map_err(|e| e.to_string())?;
    let arg = std::env::args().nth(1).unwrap_or_default();
    if arg == "--recover" { return tornado_shell::recover_pending(); }
    if arg == "--observe" {
        let first = tornado_shell::inspect_snapshot()?;
        let mut previous = tornado_shell::inspect_snapshot()?;
        let mut changed_frames = 0;
        let mut max_step = 0;
        for _ in 0..40 {
            thread::sleep(Duration::from_millis(100));
            let current = tornado_shell::inspect_snapshot()?;
            for old in &previous.items {
                if let Some(now) = current.items.iter().find(|item| item.id == old.id) {
                    let step = (now.x - old.x).abs() + (now.y - old.y).abs();
                    if step > 0 { changed_frames += 1; max_step = max_step.max(step); }
                }
            }
            previous = current;
        }
        let displaced = first.items.iter().filter(|old| previous.items.iter().any(|item|
            item.id == old.id && (item.x != old.x || item.y != old.y))).count();
        println!("Observed {changed_frames} icon moves in 4 seconds; {displaced} displaced; largest step {max_step}px");
        return Ok(());
    }
    if arg.is_empty() {
        let (count, flags, options) = tornado_shell::inspect_desktop()?;
        println!("Desktop readable: {count} icons, flags={flags:#x}, options={options:#x}");
        return Ok(());
    }
    if arg == "--interrupt" {
        let mut effect = tornado_shell::DesktopTornado::start()?;
        for _ in 0..100 {
            effect.step(0.05)?;
            thread::sleep(Duration::from_millis(10));
        }
        // Simulate sudden process termination: no destructors are run.
        std::process::exit(0);
    }
    if arg != "--exercise-recovery" { return Err("unknown diagnostic option".into()); }
    let mut before = tornado_shell::inspect_snapshot()?;
    let child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
        .arg("--interrupt").status();
    let during = tornado_shell::inspect_snapshot();
    // Always try recovery, including when the child process fails.
    let recovery = tornado_shell::recover_pending();
    let mut after = tornado_shell::inspect_snapshot()?;
    recovery?;
    let status = child.map_err(|e| e.to_string())?;
    if !status.success() { return Err(format!("movement process failed: {status}")); }
    let during = during?;
    let moved = before.items.iter().filter(|old| during.items.iter().any(|item|
        item.id == old.id && (item.x != old.x || item.y != old.y))).count();
    before.items.sort_by(|a, b| a.id.cmp(&b.id));
    after.items.sort_by(|a, b| a.id.cmp(&b.id));
    if before != after { return Err("desktop layout or view settings differ after recovery".into()); }
    if moved == 0 { return Err("no icons moved during the test".into()); }
    println!("Moved {moved} icons; restored all {} positions and view settings after simulated crash", before.items.len());
    Ok(())
}
fn main() {
    if let Err(error) = run() { eprintln!("Diagnostic failed: {error}"); std::process::exit(1); }
}
