#![forbid(unsafe_code)]

use std::fs;
use std::io::Write;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use cs2::{DeadlockedStyleOffsets, HostCs2Runtime, LocalPlayerRecord, RadarPlayerRecord};
use fun_trigger::{TriggerFrame, socket_path_from_env};
use memreader_client::resolve_host_process_by_name;
use tracing::{info, warn};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Angles {
    pitch: f32,
    yaw: f32,
}

impl Angles {
    fn new(pitch: f32, yaw: f32) -> Self {
        Self { pitch, yaw }
    }

    fn magnitude(self) -> f32 {
        (self.pitch * self.pitch + self.yaw * self.yaw).sqrt()
    }

    fn normalize(self) -> Self {
        Self {
            pitch: normalize_pitch(self.pitch),
            yaw: normalize_yaw(self.yaw),
        }
    }

    fn delta_to(self, target: Angles) -> Angles {
        Angles {
            pitch: shortest_angle_deg(target.pitch - self.pitch),
            yaw: shortest_angle_deg(target.yaw - self.yaw),
        }
    }
}

fn normalize_pitch(pitch: f32) -> f32 {
    let mut p = pitch % 360.0;
    if p > 180.0 {
        p -= 360.0;
    } else if p < -180.0 {
        p += 360.0;
    }
    p.clamp(-90.0, 90.0)
}

fn normalize_yaw(mut yaw: f32) -> f32 {
    while yaw > 180.0 {
        yaw -= 360.0;
    }
    while yaw < -180.0 {
        yaw += 360.0;
    }
    yaw
}

fn shortest_angle_deg(mut angle: f32) -> f32 {
    angle %= 360.0;
    if angle > 180.0 {
        angle -= 360.0;
    } else if angle < -180.0 {
        angle += 360.0;
    }
    angle
}

fn vector_angles(from: [f32; 3], to: [f32; 3]) -> Angles {
    let delta_x = to[0] - from[0];
    let delta_y = to[1] - from[1];
    let delta_z = to[2] - from[2];
    if delta_x.abs() < 1e-6 && delta_y.abs() < 1e-6 {
        let pitch = if delta_z > 0.0 { -90.0 } else { 90.0 };
        return Angles::new(pitch, 0.0);
    }
    let yaw = delta_y.atan2(delta_x).to_degrees();
    let horizontal_dist = (delta_x * delta_x + delta_y * delta_y).sqrt();
    let pitch = (-delta_z).atan2(horizontal_dist).to_degrees();
    Angles::new(pitch, yaw).normalize()
}

struct Config {
    socket_path: PathBuf,
    reader_hz: u64,
    head_only: bool,
    allow_teammates: bool,
    flash_check: bool,
    scope_check: bool,
    velocity_check: bool,
    velocity_threshold: f32,
}

struct ActiveProcess {
    runtime: HostCs2Runtime,
    offsets: DeadlockedStyleOffsets,
    global_vars_ptr: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct TriggerDecisionOutcome {
    should_fire: bool,
    reason: &'static str,
    crosshair_entity_index: i32,
    target_steam_id: Option<u64>,
    fov: Option<f32>,
    head_radius_fov: Option<f32>,
}

fn main() -> Result<()> {
    let _ = shared_logging::init("info");
    let config = Config::from_env_args()?;
    let socket_path = config.socket_path.clone();
    let latest = Arc::new(Mutex::new(TriggerFrame::booting()));
    let latest_for_thread = Arc::clone(&latest);

    thread::spawn(move || reader_loop(config, latest_for_thread));
    serve_loop(socket_path, latest)
}

impl Config {
    fn from_env_args() -> Result<Self> {
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--help" | "-h" => {
                    print!("{}", usage());
                    std::process::exit(0);
                }
                other => bail!("unsupported argument `{other}`\n\n{}", usage()),
            }
        }

        let reader_hz = std::env::var("FUN_TRIGGER_READER_HZ")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(120);
        Ok(Self {
            socket_path: socket_path_from_env(),
            reader_hz,
            head_only: env_flag("FUN_TRIGGER_HEAD_ONLY", false),
            allow_teammates: env_flag("FUN_TRIGGER_ALLOW_TEAMMATES", false),
            flash_check: env_flag("FUN_TRIGGER_FLASH_CHECK", false),
            scope_check: env_flag("FUN_TRIGGER_SCOPE_CHECK", false),
            velocity_check: env_flag("FUN_TRIGGER_VELOCITY_CHECK", false),
            velocity_threshold: std::env::var("FUN_TRIGGER_VELOCITY_THRESHOLD")
                .ok()
                .and_then(|value| value.parse::<f32>().ok())
                .unwrap_or(100.0),
        })
    }
}

fn usage() -> &'static str {
    "usage:\n  fun-trigger\n\nnotes:\n  reads gameplay state directly from the deadlocked-kmod reader lane,\n  computes trigger should_fire, and serves the latest decision over a Unix socket.\n"
}

fn env_flag(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn serve_loop(socket_path: PathBuf, latest: Arc<Mutex<TriggerFrame>>) -> Result<()> {
    if socket_path.exists() {
        fs::remove_file(&socket_path)
            .with_context(|| format!("remove {}", socket_path.display()))?;
    }
    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("bind {}", socket_path.display()))?;
    info!(
        socket = %socket_path.display(),
        "fun-trigger serving latest frames over unix socket"
    );

    loop {
        let (mut stream, _) = listener
            .accept()
            .with_context(|| format!("accept {}", socket_path.display()))?;
        let frame = latest
            .lock()
            .map_err(|_| anyhow!("trigger latest state lock poisoned"))?
            .clone();
        if let Err(err) = serde_json::to_writer(&mut stream, &frame) {
            warn!(error = %err, "fun-trigger client disconnected during frame write");
            continue;
        }
        if let Err(err) = stream.write_all(b"\n") {
            warn!(error = %err, "fun-trigger client disconnected during frame flush");
            continue;
        }
    }
}

fn reader_loop(config: Config, latest: Arc<Mutex<TriggerFrame>>) {
    let tick_ms = (1000 / config.reader_hz.max(1)).max(1);
    let read_period = Duration::from_millis(tick_ms);
    let retry_period = Duration::from_secs(1);
    let mut tick = 0u64;
    let mut last_decision: Option<(bool, Option<String>)> = None;
    let mut active: Option<ActiveProcess> = None;

    loop {
        if active.is_none() {
            match connect_process() {
                Ok(process) => {
                    info!(pid = process.runtime.pid(), "fun-trigger connected to cs2");
                    active = Some(process);
                }
                Err(err) => {
                    replace_latest(
                        &latest,
                        TriggerFrame::unavailable(format!("trigger connect failed: {err:#}")),
                    );
                    thread::sleep(retry_period);
                    continue;
                }
            }
        }

        let Some(process) = active.as_mut() else {
            thread::sleep(retry_period);
            continue;
        };

        if process_changed(process) {
            info!(
                old_pid = process.runtime.pid(),
                "fun-trigger detected cs2 process change; reconnecting"
            );
            active = None;
            thread::sleep(retry_period);
            continue;
        }

        match read_trigger_frame(process, &config, tick) {
            Ok(frame) => {
                tick = frame.tick;
                let decision_signature = (frame.should_fire, frame.reason.clone());
                if last_decision.as_ref() != Some(&decision_signature) {
                    info!(
                        tick = frame.tick,
                        state = if frame.should_fire {
                            "active"
                        } else {
                            "inactive"
                        },
                        reason = frame.reason.as_deref().unwrap_or("<none>"),
                        local_health = frame.local_health.unwrap_or(-1),
                        local_team_num = frame.local_team_num.unwrap_or(-1),
                        local_life_state = frame.local_life_state.unwrap_or(-1),
                        local_deathmatch_immunity =
                            frame.local_deathmatch_immunity.unwrap_or(false),
                        crosshair_entity_index = frame.crosshair_entity_index.unwrap_or(-1),
                        target_steam_id = frame.target_steam_id,
                        fov = frame.fov,
                        head_radius_fov = frame.head_radius_fov,
                        map_name = frame.map_name.as_deref().unwrap_or("<unknown>"),
                        "fun-trigger fire decision state changed"
                    );
                    last_decision = Some(decision_signature);
                }
                replace_latest(&latest, frame);
                thread::sleep(read_period);
            }
            Err(err) => {
                warn!(
                    pid = process.runtime.pid(),
                    error = %err,
                    "fun-trigger deadlocked-kmod read failed; reconnecting"
                );
                last_decision = None;
                replace_latest(
                    &latest,
                    TriggerFrame::unavailable(format!("trigger read failed: {err:#}")),
                );
                active = None;
                thread::sleep(retry_period);
            }
        }
    }
}

fn read_trigger_frame(
    process: &mut ActiveProcess,
    config: &Config,
    tick: u64,
) -> Result<TriggerFrame> {
    let snapshot = process
        .runtime
        .read_snapshot_with_deadlocked_offsets(&process.offsets, process.global_vars_ptr)?;
    let snapshot_state = cs2::classify_snapshot(&snapshot);
    let map_name = snapshot.map_name.clone();
    let local = process.runtime.read_local_player_record(&process.offsets)?;
    let has_live_local = local
        .as_ref()
        .is_some_and(|local| matches!(local.team_num, 2 | 3));

    if snapshot_state.is_in_menu && !has_live_local {
        return Ok(TriggerFrame {
            tick: tick.wrapping_add(1),
            should_fire: false,
            map_name,
            reason: Some(String::from("menu_or_no_world")),
            local_health: None,
            local_team_num: None,
            local_life_state: None,
            local_deathmatch_immunity: None,
            crosshair_entity_index: None,
            target_steam_id: None,
            fov: None,
            head_radius_fov: None,
            error: None,
        });
    }

    let Some(map_name) = map_name else {
        return Ok(TriggerFrame::unavailable("trigger missing map name"));
    };

    let radar_players = process
        .runtime
        .read_radar_player_records(&process.offsets)
        .context("read deadlocked-kmod radar players")?;
    let Some(local) = local else {
        return Ok(TriggerFrame {
            tick: tick.wrapping_add(1),
            should_fire: false,
            map_name: Some(map_name),
            reason: Some(String::from("local_player_missing")),
            local_health: None,
            local_team_num: None,
            local_life_state: None,
            local_deathmatch_immunity: None,
            crosshair_entity_index: None,
            target_steam_id: None,
            fov: None,
            head_radius_fov: None,
            error: None,
        });
    };
    let outcome = evaluate_trigger_deadlocked(process, config, &local, &radar_players);

    Ok(TriggerFrame {
        tick: tick.wrapping_add(1),
        should_fire: outcome.should_fire,
        map_name: Some(map_name),
        reason: Some(outcome.reason.to_string()),
        local_health: Some(local.health),
        local_team_num: Some(local.team_num),
        local_life_state: Some(local.life_state),
        local_deathmatch_immunity: Some(local.deathmatch_immunity),
        crosshair_entity_index: Some(outcome.crosshair_entity_index),
        target_steam_id: outcome.target_steam_id,
        fov: outcome.fov,
        head_radius_fov: outcome.head_radius_fov,
        error: None,
    })
}

fn evaluate_trigger_deadlocked(
    process: &mut ActiveProcess,
    config: &Config,
    local: &LocalPlayerRecord,
    players: &[RadarPlayerRecord],
) -> TriggerDecisionOutcome {
    let reject = |reason: &'static str| TriggerDecisionOutcome {
        should_fire: false,
        reason,
        crosshair_entity_index: local.crosshair_entity_index,
        target_steam_id: None,
        fov: None,
        head_radius_fov: None,
    };
    if !matches!(local.team_num, 2 | 3) {
        return reject("local_wrong_team");
    }
    if local.health <= 0 || local.life_state != 0 || local.deathmatch_immunity {
        return reject("local_invalid");
    }
    if config.flash_check && local.flash_duration > 0.0 {
        return reject("local_flashed");
    }
    if config.scope_check && !local.is_scoped {
        return reject("local_unscoped");
    }
    if config.velocity_check && vec3_length(local.velocity) > config.velocity_threshold {
        return reject("local_moving");
    }

    let crosshair_index = local.crosshair_entity_index;
    if crosshair_index < 0 {
        return reject("crosshair_entity_missing");
    }
    let Ok(Some(target_pawn)) = process
        .runtime
        .resolve_entity_index(&process.offsets, crosshair_index as u32)
    else {
        return reject("crosshair_entity_unresolved");
    };

    let Some(player) = players.iter().find(|player| player.pawn == target_pawn) else {
        return reject("crosshair_target_not_found");
    };
    if player.is_local
        || player.dormant
        || player.deathmatch_immunity
        || player.health <= 0
        || player.life_state != 0
    {
        return TriggerDecisionOutcome {
            should_fire: false,
            reason: "target_invalid",
            crosshair_entity_index: crosshair_index,
            target_steam_id: Some(player.steam_id),
            fov: None,
            head_radius_fov: None,
        };
    }
    if !matches!(player.team_num, 2 | 3) {
        return TriggerDecisionOutcome {
            should_fire: false,
            reason: "target_wrong_team",
            crosshair_entity_index: crosshair_index,
            target_steam_id: Some(player.steam_id),
            fov: None,
            head_radius_fov: None,
        };
    }
    if !config.allow_teammates && player.team_num == local.team_num {
        return TriggerDecisionOutcome {
            should_fire: false,
            reason: "target_teammate",
            crosshair_entity_index: crosshair_index,
            target_steam_id: Some(player.steam_id),
            fov: None,
            head_radius_fov: None,
        };
    }
    if !config.head_only {
        return TriggerDecisionOutcome {
            should_fire: true,
            reason: "fire",
            crosshair_entity_index: crosshair_index,
            target_steam_id: Some(player.steam_id),
            fov: None,
            head_radius_fov: None,
        };
    }

    let Ok(Some(head)) = process
        .runtime
        .read_bone_position(&process.offsets, player.pawn, 7)
    else {
        return TriggerDecisionOutcome {
            should_fire: false,
            reason: "head_bone_missing",
            crosshair_entity_index: crosshair_index,
            target_steam_id: Some(player.steam_id),
            fov: None,
            head_radius_fov: None,
        };
    };
    let target_angle = vector_angles(local.eye_position, head);
    let view_angles = Angles::new(local.view_angles[0], local.view_angles[1]);
    let fov = view_angles.delta_to(target_angle).magnitude();
    let distance = distance_3d(local.origin, player.origin);
    if distance <= 0.001 {
        return TriggerDecisionOutcome {
            should_fire: false,
            reason: "target_distance_invalid",
            crosshair_entity_index: crosshair_index,
            target_steam_id: Some(player.steam_id),
            fov: Some(fov),
            head_radius_fov: None,
        };
    }
    let head_radius_fov = 3.5 / distance * 100.0;
    if fov > head_radius_fov {
        return TriggerDecisionOutcome {
            should_fire: false,
            reason: "head_fov_miss",
            crosshair_entity_index: crosshair_index,
            target_steam_id: Some(player.steam_id),
            fov: Some(fov),
            head_radius_fov: Some(head_radius_fov),
        };
    }
    TriggerDecisionOutcome {
        should_fire: true,
        reason: "fire",
        crosshair_entity_index: crosshair_index,
        target_steam_id: Some(player.steam_id),
        fov: Some(fov),
        head_radius_fov: Some(head_radius_fov),
    }
}

fn vec3_length(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn distance_3d(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

fn connect_process() -> Result<ActiveProcess> {
    let mut runtime = HostCs2Runtime::attach_host_cs2()?;
    let offsets = runtime.find_deadlocked_style_offsets()?;
    let global_vars_ptr = runtime.find_global_vars_ptr()?;
    Ok(ActiveProcess {
        runtime,
        offsets,
        global_vars_ptr,
    })
}

fn process_changed(process: &ActiveProcess) -> bool {
    match resolve_host_process_by_name("cs2") {
        Ok(current) => {
            current.pid != process.runtime.pid()
                || current.start_time_ticks != process.runtime.start_time_ticks()
        }
        Err(_) => true,
    }
}

fn replace_latest(latest: &Arc<Mutex<TriggerFrame>>, frame: TriggerFrame) {
    match latest.lock() {
        Ok(mut guard) => *guard = frame,
        Err(_) => warn!("trigger latest state lock poisoned; dropping update"),
    }
}
