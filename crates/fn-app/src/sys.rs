//! The ONLY module of `fn-app` that contains `unsafe` code (the crate lint is `deny(unsafe_code)`, this module opts out; a test in `diag.rs`
//! fails if `unsafe` shows up in any other source file).
//!
//! Why it exists: a handful of Win32 facts that neither std nor winit expose, each a single call through the `windows-sys` *bindings* crate (no C
//! code, no import library beyond the system DLLs):
//! * `MessageBoxW`: native error dialogs (missing assets, panics, window / GPU failures) for a GUI-subsystem exe that has no console;
//! * `AttachConsole` + `SetStdHandle`: a GUI-subsystem release exe still prints to the terminal it was started from, and `eprintln!` can be teed into
//!   the log file (`GetStdHandle` / `SetStdHandle` swap the process stderr for an anonymous pipe that a thread copies out);
//! * `ClipCursor(NULL)` / `GetClipCursor` / `GetCursorPos` / `GetForegroundWindow`: the old runtime's mouse-capture policy needs an unconditional release
//!   (winit skips its own release while the window is unfocused) and the capture self-audit needs to read the real clip rectangle back;
//! * `GetMonitorInfoW`: the monitor work area (the taskbar is not a guess any more).
//!
//! Everything public here is safe to call; every `unsafe` block states why it is sound. Non-Windows builds get inert stubs so the crate still compiles.
#![allow(unsafe_code)]

/// `(left, top, right, bottom)` in screen pixels (Win32 `RECT` order, right / bottom exclusive).
pub type Rect = (i32, i32, i32, i32);

/// How the process ended up with a usable stderr at start-up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Console {
    /// The parent gave us valid standard handles (console subsystem, or a GUI exe started with redirected output).
    Inherited,
    /// A GUI-subsystem exe started from a terminal: attached to the parent's console and pointed stdout / stderr at it.
    Attached,
    /// No console and no handles (double click): output only goes to the log file.
    None,
}

#[cfg(windows)]
mod imp {
    use super::{Console, Rect};
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Condvar, Mutex};
    use std::os::windows::io::{AsRawHandle, FromRawHandle, IntoRawHandle};
    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO};
    use windows_sys::Win32::Storage::FileSystem::{GetFileType, FILE_TYPE_UNKNOWN};
    use windows_sys::Win32::System::Console::{AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{ClipCursor, GetClipCursor, GetCursorPos, GetForegroundWindow, MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_TOPMOST};

    fn wide(s: &str) -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() }
    fn rect(r: RECT) -> Rect { (r.left, r.top, r.right, r.bottom) }

    /// Modal native error box (topmost, foreground). The cursor clip is lifted first so the box can always be clicked.
    pub fn message_box(title: &str, text: &str) {
        release_cursor_clip();
        let (t, c) = (wide(title), wide(text));
        // SAFETY: both buffers are NUL-terminated UTF-16 that outlive the call; a null owner window is allowed by MessageBoxW.
        unsafe { MessageBoxW(0, c.as_ptr(), t.as_ptr(), MB_OK | MB_ICONERROR | MB_TOPMOST | MB_SETFOREGROUND); }
    }

    fn std_handle_ok(which: u32) -> bool {
        // SAFETY: GetStdHandle / GetFileType only read process state and tolerate NULL / INVALID_HANDLE_VALUE.
        unsafe { let h = GetStdHandle(which); h != 0 && h != INVALID_HANDLE_VALUE && GetFileType(h) != FILE_TYPE_UNKNOWN }
    }

    /// Makes stdout / stderr usable: keeps inherited handles, else attaches to the parent console (terminal launch), else leaves them unset.
    pub fn console_init() -> Console {
        if std_handle_ok(STD_ERROR_HANDLE) { return Console::Inherited; }
        // SAFETY: AttachConsole has no pointer arguments; failure (no parent console) just returns 0.
        if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 { return Console::None; }
        let open = || std::fs::OpenOptions::new().read(true).write(true).open("CONOUT$").ok();
        let Some(out) = open() else { return Console::None };
        let h = out.into_raw_handle() as HANDLE; // intentionally leaked: it is the process's stdout / stderr from now on
        // SAFETY: `h` is a valid, open console output handle that is never closed.
        unsafe { SetStdHandle(STD_OUTPUT_HANDLE, h); SetStdHandle(STD_ERROR_HANDLE, h); }
        Console::Attached
    }

    /// Marker written through the pipe by [`tee_flush`]; the tee thread swallows it and signals [`FLUSHED`].
    const MARK: &[u8] = b"\x1fFN-TEE-FLUSH\x1f";
    static FLUSHED: (Mutex<u64>, Condvar) = (Mutex::new(0), Condvar::new());
    static TEE_ACTIVE: AtomicBool = AtomicBool::new(false);

    fn find(hay: &[u8], needle: &[u8]) -> Option<usize> { hay.windows(needle.len()).position(|w| w == needle) }

    /// Replaces the process stderr by an anonymous pipe and starts a thread that hands every chunk to `sink` and copies it to the previous stderr
    /// (the terminal, a redirected file, ...). Returns false when the pipe could not be installed (stderr then stays as it was).
    /// Call [`tee_flush`] before the process exits, otherwise the last lines can still be in the pipe.
    pub fn tee_stderr(mut sink: impl FnMut(&[u8]) + Send + 'static) -> bool {
        let Ok((mut reader, writer)) = std::io::pipe() else { return false };
        // SAFETY: reads the current handle; ownership stays with the system / the parent, we only borrow it for writing below.
        let orig: HANDLE = if std_handle_ok(STD_ERROR_HANDLE) { unsafe { GetStdHandle(STD_ERROR_HANDLE) } } else { 0 };
        let wh = writer.as_raw_handle() as HANDLE;
        // SAFETY: `wh` is a valid pipe write end; the writer is leaked below so the handle stays open for the life of the process.
        if unsafe { SetStdHandle(STD_ERROR_HANDLE, wh) } == 0 { return false; }
        std::mem::forget(writer);
        let spawned = std::thread::Builder::new().name("stderr-tee".into()).spawn(move || {
            // SAFETY: a non-owning view of the old stderr handle (ManuallyDrop: never closed here).
            let mut orig_file = (orig != 0).then(|| std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_handle(orig as _) }));
            let mut forward = |bytes: &[u8]| { if bytes.is_empty() { return; } sink(bytes); if let Some(f) = orig_file.as_mut() { let _ = f.write_all(bytes); let _ = f.flush(); } };
            let (mut buf, mut pending) = ([0u8; 4096], Vec::<u8>::new());
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => pending.extend_from_slice(&buf[..n]),
                }
                loop {
                    if let Some(i) = find(&pending, MARK) {
                        forward(&pending[..i]);
                        pending.drain(..i + MARK.len());
                        *FLUSHED.0.lock().unwrap_or_else(|e| e.into_inner()) += 1; FLUSHED.1.notify_all();
                        continue;
                    }
                    // everything except a tail that could still become the marker
                    let keep = (1..MARK.len().min(pending.len() + 1)).rev().find(|&k| pending.ends_with(&MARK[..k])).unwrap_or(0);
                    let upto = pending.len() - keep;
                    forward(&pending[..upto]); pending.drain(..upto);
                    break;
                }
            }
        });
        let ok = spawned.is_ok();
        TEE_ACTIVE.store(ok, Ordering::Relaxed);
        ok
    }

    /// Blocks (at most `timeout`) until everything written to stderr so far has been copied out by the tee thread. No-op without a tee.
    pub fn tee_flush(timeout: std::time::Duration) {
        if !TEE_ACTIVE.load(Ordering::Relaxed) { return; }
        let want = *FLUSHED.0.lock().unwrap_or_else(|e| e.into_inner()) + 1;
        let mut err = std::io::stderr();
        if err.write_all(MARK).is_err() { return; }
        let _ = err.flush();
        let guard = FLUSHED.0.lock().unwrap_or_else(|e| e.into_inner());
        let _ = FLUSHED.1.wait_timeout_while(guard, timeout, |n| *n < want);
    }
    /// Unconditional `ClipCursor(NULL)`: the pointer may move over the whole desktop again.
    pub fn release_cursor_clip() {
        // SAFETY: a null RECT pointer is the documented way to remove the clip.
        unsafe { ClipCursor(std::ptr::null()); }
    }
    /// The current clip rectangle (the whole virtual desktop when nothing is clipped).
    pub fn clip_rect() -> Option<Rect> {
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        // SAFETY: `r` is a valid out pointer.
        (unsafe { GetClipCursor(&mut r) } != 0).then(|| rect(r))
    }
    pub fn cursor_pos() -> Option<(i32, i32)> {
        let mut p = POINT { x: 0, y: 0 };
        // SAFETY: `p` is a valid out pointer.
        (unsafe { GetCursorPos(&mut p) } != 0).then_some((p.x, p.y))
    }
    /// The foreground window handle (0 = none).
    pub fn foreground_window() -> isize {
        // SAFETY: no arguments.
        unsafe { GetForegroundWindow() }
    }
    /// Work area (monitor minus taskbar) of an `HMONITOR`.
    pub fn monitor_work_area(hmonitor: isize) -> Option<Rect> {
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, rcMonitor: RECT { left: 0, top: 0, right: 0, bottom: 0 }, rcWork: RECT { left: 0, top: 0, right: 0, bottom: 0 }, dwFlags: 0 };
        // SAFETY: `mi` is a valid MONITORINFO with cbSize set; an invalid HMONITOR makes the call fail, which is handled.
        (unsafe { GetMonitorInfoW(hmonitor, &mut mi) } != 0).then(|| rect(mi.rcWork))
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{Console, Rect};
    pub fn message_box(_title: &str, _text: &str) {}
    pub fn console_init() -> Console { Console::Inherited }
    pub fn tee_stderr(_sink: impl FnMut(&[u8]) + Send + 'static) -> bool { false }
    pub fn tee_flush(_timeout: std::time::Duration) {}
    pub fn release_cursor_clip() {}
    pub fn clip_rect() -> Option<Rect> { None }
    pub fn cursor_pos() -> Option<(i32, i32)> { None }
    pub fn foreground_window() -> isize { 0 }
    pub fn monitor_work_area(_hmonitor: isize) -> Option<Rect> { None }
}

pub use imp::*;
