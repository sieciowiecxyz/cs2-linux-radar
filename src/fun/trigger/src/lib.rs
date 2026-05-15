#![forbid(unsafe_code)]

use std::env;
use std::io::Read;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerFrame {
    pub tick: u64,
    pub should_fire: bool,
    pub map_name: Option<String>,
    pub reason: Option<String>,
    pub local_health: Option<i32>,
    pub local_team_num: Option<i32>,
    pub local_life_state: Option<i32>,
    pub local_deathmatch_immunity: Option<bool>,
    pub crosshair_entity_index: Option<i32>,
    pub target_steam_id: Option<u64>,
    pub fov: Option<f32>,
    pub head_radius_fov: Option<f32>,
    pub error: Option<String>,
}

impl TriggerFrame {
    pub fn booting() -> Self {
        Self {
            tick: 0,
            should_fire: false,
            map_name: None,
            reason: Some(String::from("booting")),
            local_health: None,
            local_team_num: None,
            local_life_state: None,
            local_deathmatch_immunity: None,
            crosshair_entity_index: None,
            target_steam_id: None,
            fov: None,
            head_radius_fov: None,
            error: Some(String::from("trigger booting")),
        }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            tick: 0,
            should_fire: false,
            map_name: None,
            reason: Some(String::from("unavailable")),
            local_health: None,
            local_team_num: None,
            local_life_state: None,
            local_deathmatch_immunity: None,
            crosshair_entity_index: None,
            target_steam_id: None,
            fov: None,
            head_radius_fov: None,
            error: Some(message.into()),
        }
    }
}

pub fn socket_path_from_env() -> PathBuf {
    env::var_os("FUN_TRIGGER_SOCK")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .map(|runtime_dir| runtime_dir.join("fun-trigger.sock"))
                .unwrap_or_else(|| PathBuf::from("/tmp/fun-trigger.sock"))
        })
}

pub fn read_latest_frame(socket_path: &Path) -> Result<TriggerFrame> {
    let mut stream = UnixStream::connect(socket_path)
        .with_context(|| format!("connect {}", socket_path.display()))?;
    let mut raw = String::new();
    stream
        .read_to_string(&mut raw)
        .with_context(|| format!("read {}", socket_path.display()))?;
    serde_json::from_str(&raw).context("decode trigger frame")
}
