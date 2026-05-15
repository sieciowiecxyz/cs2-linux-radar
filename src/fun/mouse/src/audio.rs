use std::fs;
use std::io::{self, Write};
use std::process::Command;
use std::thread;
use std::time::Duration;

use tracing::warn;

#[derive(Debug, Clone, Copy)]
pub enum ToggleSound {
    Enabled,
    Disabled,
}

impl ToggleSound {
    fn beep_count(self) -> usize {
        match self {
            Self::Enabled => 3,
            Self::Disabled => 1,
        }
    }
}

pub fn play_toggle_async(sound: ToggleSound) {
    thread::spawn(move || {
        if let Err(err) = play_toggle(sound) {
            warn!(error = %err, ?sound, "failed to play toggle sound");
        }
    });
}

fn play_toggle(sound: ToggleSound) -> io::Result<()> {
    let count = sound.beep_count();
    for idx in 0..count {
        if !play_once()? {
            play_terminal_bell()?;
        }
        if idx + 1 < count {
            thread::sleep(Duration::from_millis(120));
        }
    }
    Ok(())
}

fn play_once() -> io::Result<bool> {
    for backend in backends() {
        match backend.invoke() {
            Ok(true) => return Ok(true),
            Ok(false) => continue,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        }
    }
    Ok(false)
}

fn play_terminal_bell() -> io::Result<()> {
    let mut stderr = io::stderr().lock();
    stderr.write_all(b"\x07")?;
    stderr.flush()
}

fn backends() -> Vec<PlayerBackend> {
    let mut backends = Vec::new();

    if let Some(path) = existing_sound_path() {
        backends.push(PlayerBackend {
            program: "paplay",
            args: vec![path.clone()],
        });
        backends.push(PlayerBackend {
            program: "pw-play",
            args: vec![path],
        });
    }

    backends.push(PlayerBackend {
        program: "canberra-gtk-play",
        args: vec!["-i".to_string(), "bell".to_string()],
    });

    backends
}

fn existing_sound_path() -> Option<String> {
    const CANDIDATES: &[&str] = &[
        "/usr/share/sounds/freedesktop/stereo/bell.oga",
        "/usr/share/sounds/freedesktop/stereo/complete.oga",
        "/usr/share/sounds/alsa/Front_Center.wav",
    ];

    CANDIDATES
        .iter()
        .find(|path| fs::metadata(path).is_ok())
        .map(|path| (*path).to_string())
}

struct PlayerBackend {
    program: &'static str,
    args: Vec<String>,
}

impl PlayerBackend {
    fn invoke(&self) -> io::Result<bool> {
        let status = Command::new(self.program).args(&self.args).status()?;
        Ok(status.success())
    }
}
