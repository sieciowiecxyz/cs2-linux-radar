use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use evdev::{Device, KeyCode, RelativeAxisCode};
use tracing::info;

pub fn discover_mouse_paths() -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();

    for entry in fs::read_dir("/dev/input").context("read /dev/input")? {
        let entry = entry?;
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !file_name.starts_with("event") {
            continue;
        }
        let Ok(device) = Device::open(&path) else {
            continue;
        };
        let device_name = device.name().unwrap_or("<unnamed>");
        if is_excluded_virtual_device(device_name) {
            continue;
        }
        let has_rel_x = device
            .supported_relative_axes()
            .is_some_and(|axes| axes.contains(RelativeAxisCode::REL_X));
        let has_rel_y = device
            .supported_relative_axes()
            .is_some_and(|axes| axes.contains(RelativeAxisCode::REL_Y));
        let has_btn_left = device
            .supported_keys()
            .is_some_and(|keys| keys.contains(KeyCode::BTN_LEFT));
        if is_mouse_candidate(has_rel_x, has_rel_y, has_btn_left) {
            info!(
                path = %path.display(),
                name = device_name,
                "discovered physical mouse candidate"
            );
            paths.push(path);
        }
    }

    if paths.is_empty() {
        bail!("could not find any physical mouse devices under /dev/input/event*");
    }

    Ok(paths)
}

pub fn discover_keyboard_paths(toggle_key: KeyCode) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();

    for entry in fs::read_dir("/dev/input").context("read /dev/input")? {
        let entry = entry?;
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !file_name.starts_with("event") {
            continue;
        }
        let Ok(device) = Device::open(&path) else {
            continue;
        };
        let device_name = device.name().unwrap_or("<unnamed>");
        if is_excluded_virtual_device(device_name) {
            continue;
        }
        let supports_toggle = device
            .supported_keys()
            .is_some_and(|keys| keys.contains(toggle_key));
        let looks_like_keyboard = device.supported_keys().is_some_and(|keys| {
            keys.contains(KeyCode::KEY_A)
                || keys.contains(KeyCode::KEY_W)
                || keys.contains(KeyCode::KEY_ENTER)
        });
        if is_keyboard_candidate(supports_toggle, looks_like_keyboard) {
            info!(
                path = %path.display(),
                name = device_name,
                "discovered keyboard monitor candidate"
            );
            paths.push(path);
        }
    }

    if paths.is_empty() {
        bail!("could not find any keyboard exposing {:?}", toggle_key);
    }

    Ok(paths)
}

pub(crate) fn is_mouse_candidate(has_rel_x: bool, has_rel_y: bool, has_btn_left: bool) -> bool {
    has_rel_x && has_rel_y && has_btn_left
}

pub(crate) fn is_keyboard_candidate(supports_toggle: bool, looks_like_keyboard: bool) -> bool {
    supports_toggle && looks_like_keyboard
}

fn is_excluded_virtual_device(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("fun-mouse") || lower.contains("ydotoold")
}

#[cfg(test)]
mod tests {
    use super::{is_excluded_virtual_device, is_keyboard_candidate, is_mouse_candidate};

    #[test]
    fn mouse_candidate_needs_rel_axes_and_left_button() {
        assert!(is_mouse_candidate(true, true, true));
        assert!(!is_mouse_candidate(false, true, true));
        assert!(!is_mouse_candidate(true, false, true));
        assert!(!is_mouse_candidate(true, true, false));
    }

    #[test]
    fn keyboard_candidate_needs_f9_and_keyboard_shape() {
        assert!(is_keyboard_candidate(true, true));
        assert!(!is_keyboard_candidate(true, false));
        assert!(!is_keyboard_candidate(false, true));
    }

    #[test]
    fn excludes_virtual_devices_by_name() {
        assert!(is_excluded_virtual_device("fun-mouse"));
        assert!(is_excluded_virtual_device("ydotoold virtual device"));
        assert!(!is_excluded_virtual_device("Logitech USB Receiver Mouse"));
    }
}
