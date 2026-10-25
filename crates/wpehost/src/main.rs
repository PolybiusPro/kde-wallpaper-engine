//! Fake Windows desktop host for Wallpaper Engine under Wine.
//!
//! Wine has no Progman/WorkerW, so the engine never finds a window to attach
//! its wallpaper to and silently ignores apply/mute/pause. This creates the
//! classic shell layout the engine looks for:
//!
//!   Progman
//!   WorkerW  (holds SHELLDLL_DefView, the icon layer)
//!   WorkerW  (empty, directly below the one above: the wallpaper host)
//!
//! The last window is a visible top-level "WPE-Host" window sized to the
//! virtual screen, so the Plasma side can stream it.

#![windows_subsystem = "windows"]

use std::io::Read;
use std::process::ExitCode;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{GetStockObject, BLACK_BRUSH, HBRUSH};
use windows_sys::Win32::System::Environment::GetCommandLineW;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, PROCESS_INFORMATION, STARTUPINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, EnumChildWindows, GetClassNameW,
    GetMessageW, GetSystemMetrics, GetTopWindow, GetWindow, GetWindowRect, IsWindowVisible,
    RegisterClassW, SetTimer, SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage,
    GW_HWNDNEXT, HWND_TOP, MSG, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_SHOWNA, WM_TIMER, WNDCLASSW,
    WS_CHILD, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};

const MAX_PATH: usize = 260;
/// The title is capped where the C host capped it.
const TITLE_MAX: usize = 511;

/// Progman is asked to spawn the WorkerW pair with this message; ours already exist.
const WM_SPAWN_WORKERW: u32 = 0x052C;

unsafe extern "system" fn wndproc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if m == WM_SPAWN_WORKERW {
        return 0;
    }
    DefWindowProcW(h, m, w, l)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

unsafe fn make(
    cls: &str,
    title: &str,
    parent: HWND,
    (x, y, w, h): (i32, i32, i32, i32),
    style: u32,
    ex: u32,
) -> HWND {
    CreateWindowExW(
        ex,
        wide(cls).as_ptr(),
        wide(title).as_ptr(),
        style,
        x,
        y,
        w,
        h,
        parent,
        null_mut(),
        GetModuleHandleW(null()),
        null(),
    )
}

/// wallpaperconfig.layout from the engine's config.json: 0 per monitor,
/// 1 fit (span), 2 clone. None if it cannot be read.
fn read_layout(config: &[u16]) -> Option<i32> {
    let path = String::from_utf16(config).ok()?;
    let mut buf = Vec::new();
    std::fs::File::open(path).ok()?.take(1 << 20).read_to_end(&mut buf).ok()?;
    // The closing quote matters: "wallpaperconfigrecent" shares the prefix.
    let from = find(&buf, b"\"wallpaperconfig\"")?;
    let from = from + find(&buf[from..], b"\"layout\"")?;
    let colon = from + 8 + buf[from + 8..].iter().position(|&b| b == b':')?;
    atoi(&buf[colon + 1..])
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// C atoi: leading whitespace, optional sign, digits.
fn atoi(s: &[u8]) -> Option<i32> {
    let s = s.trim_ascii_start();
    let (neg, digits) = match s.first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let n = digits.iter().take_while(|b| b.is_ascii_digit()).count();
    if n == 0 {
        return Some(0);
    }
    let v: i32 = std::str::from_utf8(&digits[..n]).ok()?.parse().ok()?;
    Some(if neg { -v } else { v })
}

/// One window class per wallpaper type: scene, web, video, application.
fn is_render_class(cls: &str) -> bool {
    matches!(
        cls,
        "WPEDesktopDX11Window" | "WPEDesktopCEFWindow" | "WPEVideoWallpaper" | "WPEAppIntermediateWorker"
    )
}

/// What `add_child` collects while enumerating the host's child windows.
struct Rects {
    host: RECT,
    list: Vec<(i32, i32, i32, i32)>,
}

unsafe extern "system" fn add_child(c: HWND, l: LPARAM) -> i32 {
    let rects = &mut *(l as *mut Rects);
    let mut buf = [0u16; 64];
    let n = GetClassNameW(c, buf.as_mut_ptr(), 64).max(0) as usize;
    if !is_render_class(&String::from_utf16_lossy(&buf[..n])) || IsWindowVisible(c) == 0 {
        return 1;
    }
    let mut r: RECT = std::mem::zeroed();
    GetWindowRect(c, &mut r);
    rects.list.push((
        r.left - rects.host.left,
        r.top - rects.host.top,
        r.right - r.left,
        r.bottom - r.top,
    ));
    1
}

/// The bridge needs to know which monitors the engine has a wallpaper on, so the
/// Plasma side can show its own wallpaper on the rest. Publish the engine's
/// render windows in the host window's title, host-relative:
///   WPE-Host|x,y,w,h;x,y,w,h
/// In the engine's clone layout it makes a single window and mirrors it on the
/// other monitors itself, so that is flagged as "WPE-Host:clone|x,y,w,h".
unsafe fn update_title(host: HWND, config: &[u16], last: &mut String) {
    let mut title = String::from(if !config.is_empty() && read_layout(config) == Some(2) {
        "WPE-Host:clone"
    } else {
        "WPE-Host"
    });
    let mut rects = Rects { host: std::mem::zeroed(), list: Vec::new() };
    GetWindowRect(host, &mut rects.host);
    EnumChildWindows(host, Some(add_child), &mut rects as *mut Rects as LPARAM);
    for (i, (x, y, w, h)) in rects.list.iter().enumerate() {
        let part = format!("{}{x},{y},{w},{h}", if i > 0 { ';' } else { '|' });
        if title.len() + part.len() > TITLE_MAX {
            break;
        }
        title.push_str(&part);
    }
    if title != *last {
        SetWindowTextW(host, wide(&title).as_ptr());
        *last = title;
    }
}

/// Anything after our own name on the command line is the real program,
/// started only once the fake desktop exists so the engine finds it.
fn command_tail(cmd: &[u16]) -> &[u16] {
    let quote = b'"' as u16;
    let start = if cmd.first() == Some(&quote) {
        cmd[1..].iter().position(|&c| c == quote).map(|p| p + 1)
    } else {
        cmd.iter().position(|&c| c == b' ' as u16 || c == b'\t' as u16)
    };
    let Some(mut i) = start else { return &[] };
    while i < cmd.len() && cmd[i] != b' ' as u16 {
        i += 1;
    }
    while i < cmd.len() && cmd[i] == b' ' as u16 {
        i += 1;
    }
    &cmd[i..]
}

/// config.json sits next to the launcher: the first word of the command.
fn config_path(cmd: &[u16]) -> Vec<u16> {
    let quote = b'"' as u16;
    let exe = if cmd.first() == Some(&quote) { &cmd[1..] } else { cmd };
    let stop = if cmd.first() == Some(&quote) { quote } else { b' ' as u16 };
    let end = exe.iter().position(|&c| c == stop).unwrap_or(exe.len());
    let sep = exe[..end]
        .iter()
        .rposition(|&c| c == b'\\' as u16 || c == b'/' as u16)
        .map_or(0, |p| p + 1);
    if sep > 0 && sep + 12 < MAX_PATH {
        let mut path = exe[..sep].to_vec();
        path.extend("config.json".encode_utf16());
        path
    } else {
        Vec::new()
    }
}

fn main() -> ExitCode {
    unsafe { run() }
}

unsafe fn run() -> ExitCode {
    let hinst = GetModuleHandleW(null());
    for cls in ["Progman", "WorkerW", "SHELLDLL_DefView"] {
        let name = wide(cls);
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: null_mut(),
            hCursor: null_mut(),
            // A monitor with no wallpaper leaves the host window bare; show black
            // there instead of whatever stale pixels the window holds.
            hbrBackground: GetStockObject(BLACK_BRUSH) as HBRUSH,
            lpszMenuName: null(),
            lpszClassName: name.as_ptr(),
        };
        if RegisterClassW(&wc) == 0 {
            eprintln!("RegisterClass {cls} failed");
            return ExitCode::FAILURE;
        }
    }

    let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
    let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
    let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
    let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);

    let popup = WS_POPUP;
    // NOACTIVATE: Wine raises a window when it is activated, which would put
    // the wallpaper host above the icon layer and hide it from the engine.
    let tool = WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;

    let progman = make("Progman", "Program Manager", null_mut(), (vx, vy, vw, vh), popup, tool);
    // Created before the icon WorkerW so it sits below it in z-order.
    let host = make("WorkerW", "WPE-Host", null_mut(), (vx, vy, vw, vh), popup, tool);
    // The icon layer only has to exist and be findable; keep it tiny and far
    // off-screen so it never shows on the desktop.
    let icons = make("WorkerW", "", null_mut(), (vx - 10000, vy - 10000, 1, 1), popup, tool);
    let defview = make("SHELLDLL_DefView", "", icons, (0, 0, 1, 1), WS_CHILD, 0);

    if progman.is_null() || host.is_null() || icons.is_null() || defview.is_null() {
        eprintln!("window creation failed");
        return ExitCode::FAILURE;
    }

    ShowWindow(host, SW_SHOWNA);
    ShowWindow(defview, SW_SHOWNA);
    ShowWindow(icons, SW_SHOWNA);
    // ShowWindow can reorder siblings; the engine takes the WorkerW right
    // after the DefView one as the wallpaper host, so icons must stay on top.
    raise(icons);

    let mut config = Vec::new();
    let cmdline = GetCommandLineW();
    let len = (0..).take_while(|&i| *cmdline.add(i) != 0).count();
    let tail = command_tail(std::slice::from_raw_parts(cmdline, len));
    if !tail.is_empty() {
        config = config_path(tail);
        let mut buf: Vec<u16> = tail.iter().copied().take(4095).collect();
        buf.push(0);
        let si = STARTUPINFOW { cb: size_of::<STARTUPINFOW>() as u32, ..std::mem::zeroed() };
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        if CreateProcessW(null(), buf.as_mut_ptr(), null(), null(), 0, 0, null(), null(), &si, &mut pi) == 0 {
            eprintln!("CreateProcess failed");
            return ExitCode::FAILURE;
        }
        windows_sys::Win32::Foundation::CloseHandle(pi.hThread);
        windows_sys::Win32::Foundation::CloseHandle(pi.hProcess);
    }

    SetTimer(progman, 1, 200, None);

    let mut last = String::new();
    let mut msg: MSG = std::mem::zeroed();
    while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
        if msg.message == WM_TIMER && msg.wParam == 1 {
            // Something (the compositor's focus sync) raises the host; put the
            // icon layer back on top so the engine can find the host below it.
            let mut h = GetTopWindow(null_mut());
            while !h.is_null() && h != host && h != icons {
                h = GetWindow(h, GW_HWNDNEXT);
            }
            if h == host {
                raise(icons);
            }
            update_title(host, &config, &mut last);
            continue;
        }
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    ExitCode::SUCCESS
}

unsafe fn raise(h: HWND) {
    SetWindowPos(h, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn atoi_matches_c() {
        assert_eq!(atoi(b" 2,"), Some(2));
        assert_eq!(atoi(b"-1}"), Some(-1));
        assert_eq!(atoi(b"x"), Some(0));
    }

    #[test]
    fn layout_uses_the_exact_wallpaperconfig_key() {
        let cfg = r#"{"a":{"general":{"wallpaperconfigrecent":{"layout":1},"wallpaperconfig":{"x":0,"layout" : 2}}}}"#;
        let dir = std::env::temp_dir().join("wpehost-test-config.json");
        std::fs::write(&dir, cfg).unwrap();
        assert_eq!(read_layout(&w(dir.to_str().unwrap())), Some(2));
        std::fs::write(&dir, r#"{"wallpaperconfigrecent":{"layout":1}}"#).unwrap();
        assert_eq!(read_layout(&w(dir.to_str().unwrap())), None);
        std::fs::remove_file(&dir).unwrap();
    }

    #[test]
    fn tail_skips_our_own_name() {
        let t = |s: &str| String::from_utf16(command_tail(&w(s))).unwrap();
        assert_eq!(t(r#""C:\a b\wpehost.exe" "S:\x\launcher.exe" -run w.exe"#), r#""S:\x\launcher.exe" -run w.exe"#);
        assert_eq!(t(r"wpehost.exe  S:\x\launcher.exe -run"), r"S:\x\launcher.exe -run");
        assert_eq!(t("wpehost.exe"), "");
        assert_eq!(t(r#""wpehost.exe""#), "");
    }

    #[test]
    fn config_sits_next_to_the_launcher() {
        let c = |s: &str| String::from_utf16(&config_path(&w(s))).unwrap();
        assert_eq!(c(r#""S:\apps\we\launcher.exe" -run x"#), r"S:\apps\we\config.json");
        assert_eq!(c(r"S:\apps\we\launcher.exe -run x"), r"S:\apps\we\config.json");
        assert_eq!(c("launcher.exe"), "");
    }

    #[test]
    fn render_classes() {
        assert!(is_render_class("WPEVideoWallpaper"));
        assert!(!is_render_class("WPEUI"));
    }
}
