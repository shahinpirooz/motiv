//! macOS signals: on-screen window titles (CGWindowList; titles need the
//! Screen Recording permission Motiv already asks for) and which processes
//! are capturing audio input (Core Audio process objects, macOS 14.2+).

use super::detect::WindowInfo;
use core_foundation::array::CFArray;
use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use core_graphics::window::{
    copy_window_info, kCGNullWindowID, kCGWindowLayer, kCGWindowListExcludeDesktopElements,
    kCGWindowListOptionOnScreenOnly, kCGWindowName, kCGWindowOwnerName,
};

/// Normal-layer on-screen windows with their owner app name and title.
pub fn windows() -> Vec<WindowInfo> {
    let Some(list) = copy_window_info(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        kCGNullWindowID,
    ) else {
        return Vec::new();
    };
    let list: CFArray<CFDictionary<CFString, CFType>> = unsafe { CFArray::wrap_under_get_rule(list.as_concrete_TypeRef()) };
    let (k_owner, k_name, k_layer) = unsafe {
        (
            CFString::wrap_under_get_rule(kCGWindowOwnerName),
            CFString::wrap_under_get_rule(kCGWindowName),
            CFString::wrap_under_get_rule(kCGWindowLayer),
        )
    };
    let mut out = Vec::new();
    for dict in list.iter() {
        let layer = dict
            .find(&k_layer)
            .and_then(|v| v.downcast::<core_foundation::number::CFNumber>())
            .and_then(|n| n.to_i64())
            .unwrap_or(0);
        if layer != 0 {
            continue; // menu bar, overlays, etc.
        }
        let get = |k: &CFString| dict.find(k).and_then(|v| v.downcast::<CFString>()).map(|s| s.to_string());
        let title = get(&k_name).unwrap_or_default();
        if title.trim().is_empty() {
            continue;
        }
        out.push(WindowInfo { owner: get(&k_owner).unwrap_or_default(), title });
    }
    out
}

/// Processes currently capturing audio input, as "App Name (bundle.id)".
/// Same Core Audio process API Motiv uses to list apps playing audio.
#[cfg(target_os = "macos")]
pub fn mic_users() -> Option<Vec<String>> {
    use cidre::core_audio as ca;
    let own_pid = std::process::id() as i32;
    let processes = ca::System::processes().ok()?;
    let mut users = Vec::new();
    for p in processes {
        if !p.is_running_input().unwrap_or(false) {
            continue;
        }
        let pid = p.pid().ok();
        if pid.map(|p| p as i32) == Some(own_pid) {
            continue;
        }
        let bundle = p.bundle_id().ok().map(|b| b.to_string()).unwrap_or_default();
        let name = pid
            .and_then(cidre::ns::RunningApp::with_pid)
            .and_then(|app| app.localized_name().map(|s| s.to_string()))
            .unwrap_or_default();
        users.push(format!("{} ({})", name, bundle));
    }
    Some(users)
}
