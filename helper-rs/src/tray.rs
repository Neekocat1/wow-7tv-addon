//! System tray icon (native Win32: Shell_NotifyIcon), replacing pystray.
//!
//! Menu: status line, "Check 7TV for new emotes now", "Open emote folder", "Open log",
//! "Start with Windows" (checked when on), "Quit". Balloon notifications when requests finish.
//! The service runs on a background thread; the tray owns the main thread's message loop.

use crate::installer;
use crate::paths::AppPaths;
use crate::service::{Service, Shared};
use crate::win::{copy_wide, wide};
use image::RgbaImage;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, OnceLock};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject, HBITMAP,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics, HICON, ICONINFO,
    MF_CHECKED, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage,
    RegisterClassW, RegisterWindowMessageW, SM_CXSMICON, SetForegroundWindow, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP, WM_CLOSE, WM_DESTROY,
    WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
};

const WM_TRAY: u32 = WM_APP + 1;
const ICON_ID: u32 = 1;
const ID_REFRESH: usize = 1;
const ID_FOLDER: usize = 2;
const ID_LOG: usize = 3;
const ID_AUTOSTART: usize = 4;
const ID_QUIT: usize = 5;

static HWND_VALUE: AtomicIsize = AtomicIsize::new(0);
static ICON_VALUE: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_CREATED: AtomicIsize = AtomicIsize::new(0);
static CTX: OnceLock<(Arc<Shared>, AppPaths)> = OnceLock::new();

fn hwnd() -> HWND {
    HWND_VALUE.load(Ordering::SeqCst) as HWND
}

fn create_hicon(img: &RgbaImage) -> HICON {
    let (w, h) = img.dimensions();
    // SAFETY: standard DIB section + mask bitmap; pixel copy stays inside the w*h*4 buffer.
    unsafe {
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w as i32;
        bmi.bmiHeader.biHeight = -(h as i32); // top-down
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let color: HBITMAP = CreateDIBSection(
            std::ptr::null_mut(),
            &bmi,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        if color.is_null() || bits.is_null() {
            return std::ptr::null_mut();
        }
        let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
        for (i, p) in img.pixels().enumerate() {
            dst[i * 4..i * 4 + 4].copy_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
        let mask_bits = vec![0u8; (w.div_ceil(16) * 2 * h) as usize];
        let mask: HBITMAP = CreateBitmap(w as i32, h as i32, 1, 1, mask_bits.as_ptr().cast());
        let info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info);
        DeleteObject(color);
        DeleteObject(mask);
        icon
    }
}

fn base_data() -> NOTIFYICONDATAW {
    // SAFETY: NOTIFYICONDATAW is plain data; zero is a valid starting state.
    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd();
    nid.uID = ICON_ID;
    nid
}

fn add_icon(tip: &str) {
    let mut nid = base_data();
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = ICON_VALUE.load(Ordering::SeqCst) as HICON;
    copy_wide(&mut nid.szTip, tip);
    // SAFETY: nid is fully initialised.
    unsafe {
        Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

fn set_tip(tip: &str) {
    if hwnd().is_null() {
        return;
    }
    let mut nid = base_data();
    nid.uFlags = NIF_TIP;
    copy_wide(&mut nid.szTip, tip);
    // SAFETY: nid is fully initialised. Shell_NotifyIcon may be called from any thread.
    unsafe {
        Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

fn balloon(title: &str, msg: &str) {
    if hwnd().is_null() {
        return;
    }
    let mut nid = base_data();
    nid.uFlags = NIF_INFO;
    copy_wide(&mut nid.szInfo, msg);
    copy_wide(&mut nid.szInfoTitle, title);
    nid.dwInfoFlags = NIIF_INFO;
    // SAFETY: as above.
    unsafe {
        Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

fn remove_icon() {
    let nid = base_data();
    // SAFETY: as above.
    unsafe {
        Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

fn tooltip(shared: &Shared) -> String {
    format!("7TV Emotes Helper - {}", shared.status())
        .chars()
        .take(120)
        .collect()
}

fn show_menu(hwnd: HWND) {
    let Some((shared, paths)) = CTX.get() else {
        return;
    };
    // SAFETY: standard popup-menu sequence on the window's own thread.
    let cmd = unsafe {
        let menu = CreatePopupMenu();
        let status = wide(&shared.status());
        AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, status.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        let items = [
            (ID_REFRESH, "Check 7TV for new emotes now", false),
            (ID_FOLDER, "Open emote folder", false),
            (ID_LOG, "Open log", false),
            (
                ID_AUTOSTART,
                "Start with Windows",
                installer::autostart_enabled(),
            ),
        ];
        let labels: Vec<Vec<u16>> = items.iter().map(|(_, l, _)| wide(l)).collect();
        for ((id, _, checked), label) in items.iter().zip(&labels) {
            AppendMenuW(
                menu,
                MF_STRING | if *checked { MF_CHECKED } else { 0 },
                *id,
                label.as_ptr(),
            );
        }
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        let quit = wide("Quit");
        AppendMenuW(menu, MF_STRING, ID_QUIT, quit.as_ptr());
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            pt.x,
            pt.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        cmd as usize
    };
    match cmd {
        ID_REFRESH => shared.request_refresh(),
        ID_FOLDER => {
            if let Some(dir) = shared.lib_dirs.lock().unwrap().first().cloned() {
                installer::open_path(&dir);
            }
        }
        ID_LOG => installer::open_path(&paths.log_file()),
        ID_AUTOSTART => {
            if let Err(e) = installer::set_autostart(!installer::autostart_enabled()) {
                log::warn!("couldn't change autostart: {e}");
            }
        }
        ID_QUIT => {
            shared.stop();
            // SAFETY: destroying our own window from its thread.
            unsafe {
                DestroyWindow(hwnd);
            }
        }
        _ => {}
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            let ev = (lparam & 0xFFFF) as u32;
            if ev == WM_RBUTTONUP || ev == WM_LBUTTONUP {
                show_menu(hwnd);
            }
            0
        }
        WM_CLOSE => {
            // SAFETY: our own window.
            unsafe { DestroyWindow(hwnd) };
            0
        }
        WM_DESTROY => {
            remove_icon();
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            0
        }
        m if m != 0 && m as isize == TASKBAR_CREATED.load(Ordering::SeqCst) => {
            // Explorer restarted: put the icon back.
            if let Some((shared, _)) = CTX.get() {
                add_icon(&tooltip(shared));
            }
            0
        }
        // SAFETY: default handling for everything else.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Run the tray icon on this thread until Quit (or a newer helper takes over).
pub fn run_tray(mut service: Service) {
    let shared = service.shared.clone();
    let _ = CTX.set((shared.clone(), service.paths.clone()));

    // SAFETY: window class/window creation with valid, NUL-terminated strings that outlive the calls.
    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let class = wide("SevenTVEmotesHelperTray");
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = hinst;
        wc.lpszClassName = class.as_ptr();
        RegisterClassW(&wc);
        let title = wide("7TV Emotes Helper");
        let h = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        if h.is_null() {
            log::warn!("couldn't create the tray window; running without a tray icon");
            service.run();
            return;
        }
        HWND_VALUE.store(h as isize, Ordering::SeqCst);
        let tc = wide("TaskbarCreated");
        TASKBAR_CREATED.store(
            RegisterWindowMessageW(tc.as_ptr()) as isize,
            Ordering::SeqCst,
        );
        let size = (GetSystemMetrics(SM_CXSMICON).max(16)) as u32;
        ICON_VALUE.store(
            create_hicon(&crate::draw::make_icon(size)) as isize,
            Ordering::SeqCst,
        );
    }
    add_icon(&tooltip(&shared));

    *shared.notify.lock().unwrap() = Box::new(balloon);
    let s2 = shared.clone();
    *shared.on_status.lock().unwrap() = Box::new(move |_| set_tip(&tooltip(&s2)));
    *shared.on_quit.lock().unwrap() = Box::new(|| {
        // SAFETY: posting to our window from the service thread is allowed.
        unsafe {
            PostMessageW(hwnd(), WM_CLOSE, 0, 0);
        }
    });

    let worker = std::thread::Builder::new()
        .name("seventv-helper".into())
        .spawn(move || service.run());
    if let Err(e) = &worker {
        log::error!("couldn't start the helper thread: {e}");
    }

    // SAFETY: standard message loop on the thread that owns the window.
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    shared.stop();
    // Give the service a moment to finish writing files, like a clean shutdown; don't hang on
    // a long download.
    if let Ok(handle) = worker {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !handle.is_finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    log::info!("7TV Emotes Helper stopped");
}
