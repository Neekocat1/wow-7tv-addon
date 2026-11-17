//! Small Win32 helpers: dialogs, registry, shell, console. (Windows only.)

use std::ffi::c_void;
use std::path::Path;
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, HANDLE};
use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree};
use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Shell::{
    BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW, SHBrowseForFolderW,
    SHGetPathFromIDListW, ShellExecuteW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND, MessageBoxW, SW_SHOWNORMAL,
};

/// NUL-terminated UTF-16.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Copy `s` into a fixed UTF-16 buffer, truncated, NUL-terminated.
pub fn copy_wide(dst: &mut [u16], s: &str) {
    let src: Vec<u16> = s.encode_utf16().collect();
    let n = src.len().min(dst.len().saturating_sub(1));
    dst[..n].copy_from_slice(&src[..n]);
    dst[n] = 0;
}

/// Lets `--once` / `--no-tray` print to the terminal they were started from
/// (the exe is a GUI-subsystem program so it never opens a console window of its own).
pub fn attach_parent_console() {
    // SAFETY: plain Win32 call; fails harmlessly when there is no parent console.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

pub fn message_box(title: &str, text: &str) {
    let (t, m) = (wide(title), wide(text));
    // SAFETY: valid NUL-terminated strings.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            m.as_ptr(),
            t.as_ptr(),
            MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND,
        );
    }
}

/// Folder picker (the classic shell dialog). None if cancelled.
pub fn pick_folder(title: &str) -> Option<String> {
    let t = wide(title);
    let mut display = [0u16; 260];
    // SAFETY: BROWSEINFOW is plain data; all pointers outlive the call.
    unsafe {
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        let mut bi: BROWSEINFOW = std::mem::zeroed();
        bi.pszDisplayName = display.as_mut_ptr();
        bi.lpszTitle = t.as_ptr();
        bi.ulFlags = BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE;
        let pidl = SHBrowseForFolderW(&bi);
        if pidl.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let ok = SHGetPathFromIDListW(pidl, buf.as_mut_ptr());
        CoTaskMemFree(pidl as *const c_void);
        if ok == 0 {
            return None;
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..n])).filter(|s| !s.is_empty())
    }
}

pub fn shell_open(p: &Path) {
    let (verb, file) = (wide("open"), wide(&p.to_string_lossy()));
    // SAFETY: valid NUL-terminated strings.
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

pub fn reg_get_user(key: &str, name: &str) -> Option<String> {
    let (k, v) = (wide(key), wide(name));
    let mut buf = vec![0u16; 2048];
    let mut len: u32 = (buf.len() * 2) as u32;
    // SAFETY: buffer and length describe the same allocation.
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            k.as_ptr(),
            v.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if rc != 0 {
        return None;
    }
    let n = (len as usize / 2).min(buf.len());
    let s = String::from_utf16_lossy(&buf[..n]);
    Some(s.trim_end_matches('\0').to_string())
}

pub fn reg_set_user(key: &str, name: &str, value: &str) -> std::io::Result<()> {
    let (k, v, data) = (wide(key), wide(name), wide(value));
    // SAFETY: data is a NUL-terminated UTF-16 string of the stated byte length.
    let rc = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            k.as_ptr(),
            v.as_ptr(),
            REG_SZ,
            data.as_ptr().cast(),
            (data.len() * 2) as u32,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(rc as i32))
    }
}

pub fn reg_delete_user(key: &str, name: &str) -> std::io::Result<()> {
    let (k, v) = (wide(key), wide(name));
    // SAFETY: valid NUL-terminated strings.
    let rc = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr()) };
    if rc == 0 || rc == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(rc as i32))
    }
}

/// A named mutex held while the helper runs, so the installer can tell it is running
/// (Inno Setup's AppMutex / CheckForMutexes).
pub struct RunningMutex(HANDLE);

// SAFETY: a mutex handle can be closed from any thread.
unsafe impl Send for RunningMutex {}

impl Drop for RunningMutex {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: we own the handle.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

pub const RUNNING_MUTEX_NAME: &str = "SevenTVEmotesHelper_running";

pub fn running_mutex() -> RunningMutex {
    let name = wide(RUNNING_MUTEX_NAME);
    // SAFETY: valid name; null security attributes.
    RunningMutex(unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) })
}
