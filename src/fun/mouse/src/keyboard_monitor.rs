use std::io;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use evdev::{Device, EventSummary, KeyCode};
use tracing::warn;

#[derive(Debug, Clone, Copy)]
pub enum KeyboardCommand {
    ToggleRelay,
}

pub fn spawn(
    paths: Vec<PathBuf>,
    target_key: KeyCode,
    tx: Sender<KeyboardCommand>,
) -> io::Result<thread::JoinHandle<()>> {
    let mut devices = Vec::new();
    for path in paths {
        let device = Device::open(&path)?;
        device.set_nonblocking(true)?;
        devices.push(device);
    }

    Ok(thread::spawn(move || {
        loop {
            let mut idx = 0;
            while idx < devices.len() {
                let remove_device = match devices[idx].fetch_events() {
                    Ok(events) => {
                        for event in events {
                            if let EventSummary::Key(_, key, value) = event.destructure()
                                && key == target_key
                                && value == 1
                            {
                                if tx.send(KeyboardCommand::ToggleRelay).is_err() {
                                    return;
                                }
                            }
                        }
                        false
                    }
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => false,
                    Err(err) => {
                        if device_gone(&err) {
                            warn!(error = %err, "keyboard monitor lost device; dropping watcher");
                            true
                        } else {
                            warn!(error = %err, "keyboard monitor fetch_events failed");
                            false
                        }
                    }
                };
                if remove_device {
                    devices.remove(idx);
                    continue;
                }
                idx += 1;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }))
}

fn device_gone(err: &io::Error) -> bool {
    matches!(err.kind(), io::ErrorKind::NotFound) || err.raw_os_error() == Some(19)
}
