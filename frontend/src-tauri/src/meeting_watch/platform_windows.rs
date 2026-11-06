//! Windows signals: visible window titles (EnumWindows) and which apps are
//! using the microphone right now (the privacy "ConsentStore" in the
//! registry, the same data behind the taskbar microphone indicator).

use super::detect::WindowInfo;
use std::collections::HashMap;
use windows_sys::Win32::Foundation::{BOOL, ERROR_SUCCESS, FILETIME, HWND, LPARAM, TRUE};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

struct EnumState<'a> {
    names: &'a HashMap<u32, String>,
    out: Vec<WindowInfo>,
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let state = &mut *(lparam as *mut EnumState);
    if IsWindowVisible(hwnd) == 0 {
        return TRUE;
    }
    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return TRUE;
    }
    let mut buf = vec![0u16; len as usize + 1];
    let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    if n <= 0 {
        return TRUE;
    }
    let title = String::from_utf16_lossy(&buf[..n as usize]);
    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    let owner = state.names.get(&pid).cloned().unwrap_or_default();
    state.out.push(WindowInfo { owner, title });
    TRUE
}

/// Visible top-level windows with their owning process name.
pub fn windows(process_names: &HashMap<u32, String>) -> Vec<WindowInfo> {
    let mut state = EnumState { names: process_names, out: Vec::new() };
    unsafe {
        EnumWindows(Some(enum_proc), &mut state as *mut EnumState as LPARAM);
    }
    state.out
}

struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            RegCloseKey(self.0);
        }
    }
}

fn open(parent: HKEY, path: &str) -> Option<Key> {
    let mut h: HKEY = std::ptr::null_mut();
    let p = wide(path);
    let rc = unsafe { RegOpenKeyExW(parent, p.as_ptr(), 0, KEY_READ, &mut h) };
    (rc == ERROR_SUCCESS).then_some(Key(h))
}

fn subkeys(k: &Key) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0u32;
    loop {
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let mut ft = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let rc = unsafe {
            RegEnumKeyExW(k.0, i, buf.as_mut_ptr(), &mut len, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), &mut ft)
        };
        if rc != ERROR_SUCCESS {
            break;
        }
        out.push(String::from_utf16_lossy(&buf[..len as usize]));
        i += 1;
    }
    out
}

fn read_u64(k: &Key, name: &str) -> Option<u64> {
    let n = wide(name);
    let mut data = 0u64;
    let mut size = std::mem::size_of::<u64>() as u32;
    let rc = unsafe {
        RegQueryValueExW(k.0, n.as_ptr(), std::ptr::null(), std::ptr::null_mut(), &mut data as *mut u64 as *mut u8, &mut size)
    };
    (rc == ERROR_SUCCESS).then_some(data)
}

/// An app is using the mic when it has a start time and no stop time yet.
fn in_use(k: &Key) -> bool {
    read_u64(k, "LastUsedTimeStart").unwrap_or(0) > 0 && read_u64(k, "LastUsedTimeStop") == Some(0)
}

/// Names of apps using the microphone: packaged app ids (e.g.
/// "MSTeams_8wekyb3d8bbwe") and desktop exe paths (e.g.
/// "C:\Program Files\Google\Chrome\Application\chrome.exe").
pub fn mic_users() -> Option<Vec<String>> {
    const ROOT: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";
    let root = open(HKEY_CURRENT_USER, ROOT)?;
    let mut users = Vec::new();
    for name in subkeys(&root) {
        if name == "NonPackaged" {
            if let Some(np) = open(root.0, "NonPackaged") {
                for exe in subkeys(&np) {
                    if let Some(k) = open(np.0, &exe) {
                        if in_use(&k) {
                            users.push(exe.replace('#', "\\"));
                        }
                    }
                }
            }
        } else if let Some(k) = open(root.0, &name) {
            if in_use(&k) {
                users.push(name);
            }
        }
    }
    Some(users)
}
