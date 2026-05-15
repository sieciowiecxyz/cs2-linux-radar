use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cs2::{DeadlockedStyleOffsets, HostCs2Runtime, RadarPlayerRecord};
use memreader_client::resolve_host_process_by_name;
use runtime_types::Snapshot;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::map_registry::{
    MapTransform, load_map_images, load_map_transforms, normalize_map_key, world_to_radar,
};
use crate::model::{
    PlayerRelationship, RadarDebugCompareCounts, RadarDebugCompareResponse, RadarDebugCounts,
    RadarDebugPlayer, RadarDebugPlayerComparison, RadarDebugSnapshot, RadarDebugStatus,
    RadarLayerPlayer, SnapshotLayerPlayer,
};

const PROCESS_NAME: &str = "cs2";
const DEFAULT_HZ: u64 = 120;

pub type SharedSnapshot = Arc<RwLock<RadarDebugSnapshot>>;
pub type SharedCompare = Arc<RwLock<RadarDebugCompareResponse>>;

struct ActiveProcess {
    runtime: HostCs2Runtime,
    offsets: DeadlockedStyleOffsets,
    global_vars_ptr: usize,
}

#[derive(Debug, Clone, Copy)]
struct LowLevelPlayerCounts {
    server_players: u32,
    other_players: u32,
    resolved_other_players: u32,
}

pub struct ReaderState {
    pub latest: SharedSnapshot,
    pub latest_compare: SharedCompare,
}

impl ReaderState {
    pub fn new() -> Self {
        Self {
            latest: Arc::new(RwLock::new(RadarDebugSnapshot::booting())),
            latest_compare: Arc::new(RwLock::new(RadarDebugCompareResponse {
                tick: 0,
                map_key: None,
                map_image: None,
                gameplay_window: None,
                counts: RadarDebugCompareCounts {
                    low_level_server_players: None,
                    low_level_other_players: None,
                    low_level_resolved_other_players: None,
                    snapshot_other_players: 0,
                    radar_record_players: 0,
                    rendered_players: 0,
                },
                comparisons: Vec::new(),
            })),
        }
    }

    pub fn spawn(self: &Arc<Self>) {
        let latest = Arc::clone(&self.latest);
        let latest_compare = Arc::clone(&self.latest_compare);
        tokio::spawn(async move {
            let images_dir = assets_root().join("radars");
            let transforms_dir = assets_root().join("json");
            let images = match load_map_images(&images_dir) {
                Ok(value) => value,
                Err(err) => {
                    *latest.write().await = RadarDebugSnapshot::reader_unavailable(format!(
                        "load radar images: {err:#}"
                    ));
                    return;
                }
            };
            let transforms = match load_map_transforms(&transforms_dir) {
                Ok(value) => value,
                Err(err) => {
                    *latest.write().await = RadarDebugSnapshot::reader_unavailable(format!(
                        "load radar transforms: {err:#}"
                    ));
                    return;
                }
            };

            let tick_ms = (1000 / DEFAULT_HZ.max(1)).max(1);
            let read_period = Duration::from_millis(tick_ms);
            let retry_period = Duration::from_secs(1);
            let mut active: Option<ActiveProcess> = None;
            let mut tick = 0u64;

            loop {
                if active.is_none() {
                    match connect_process() {
                        Ok(process) => {
                            info!(pid = process.runtime.pid(), "fun-radar connected to cs2");
                            active = Some(process);
                        }
                        Err(err) => {
                            *latest.write().await = RadarDebugSnapshot::reader_unavailable(
                                format!("connect failed: {err:#}"),
                            );
                            tokio::time::sleep(retry_period).await;
                            continue;
                        }
                    }
                }

                let Some(process) = active.as_mut() else {
                    tokio::time::sleep(retry_period).await;
                    continue;
                };

                if process_changed(process) {
                    info!(
                        old_pid = process.runtime.pid(),
                        "fun-radar detected cs2 process change; reconnecting"
                    );
                    active = None;
                    tokio::time::sleep(retry_period).await;
                    continue;
                }

                let snapshot = match process.runtime.read_snapshot_with_deadlocked_offsets(
                    &process.offsets,
                    process.global_vars_ptr,
                ) {
                    Ok(value) => value,
                    Err(err) => {
                        warn!(pid = process.runtime.pid(), error = %err, "fun-radar read failed; reconnecting");
                        *latest.write().await =
                            RadarDebugSnapshot::reader_unavailable(format!("read failed: {err:#}"));
                        active = None;
                        tokio::time::sleep(retry_period).await;
                        continue;
                    }
                };

                let counts = None;
                let radar_players =
                    match process.runtime.read_radar_player_records(&process.offsets) {
                        Ok(value) => value,
                        Err(err) => {
                            warn!(
                                pid = process.runtime.pid(),
                                error = %err,
                                "fun-radar deadlocked-style player read failed; reconnecting"
                            );
                            *latest.write().await = RadarDebugSnapshot::reader_unavailable(
                                format!("deadlocked-style player read failed: {err:#}"),
                            );
                            active = None;
                            tokio::time::sleep(retry_period).await;
                            continue;
                        }
                    };
                tick = tick.wrapping_add(1);
                let radar_snapshot = build_snapshot(
                    tick,
                    &snapshot,
                    &radar_players,
                    counts,
                    &images,
                    &transforms,
                );
                let compare =
                    build_compare_response(&radar_snapshot, &snapshot, &radar_players, counts);
                *latest_compare.write().await = compare;
                *latest.write().await = radar_snapshot;
                tokio::time::sleep(read_period).await;
            }
        });
    }
}

fn build_snapshot(
    tick: u64,
    snapshot: &Snapshot,
    radar_players: &[RadarPlayerRecord],
    counts: Option<LowLevelPlayerCounts>,
    images: &BTreeMap<String, String>,
    transforms: &BTreeMap<String, MapTransform>,
) -> RadarDebugSnapshot {
    let gameplay_window = cs2::classify_snapshot(snapshot).phase.as_str().to_string();
    let local_team = radar_players
        .iter()
        .find(|player| player.is_local)
        .map(|player| player.team_num);
    let counts = RadarDebugCounts {
        low_level_server_players: counts.map(|value| value.server_players),
        low_level_other_players: counts.map(|value| value.other_players),
        low_level_resolved_other_players: counts.map(|value| value.resolved_other_players),
        snapshot_other_players: Some(snapshot.other_players.len() as u32),
        shown_players: 0,
    };

    let Some(raw_map_name) = snapshot.map_name.as_deref() else {
        return RadarDebugSnapshot::no_map(
            tick,
            Some(gameplay_window),
            counts,
            "no map name in snapshot",
        );
    };
    let Some(map_key) = normalize_map_key(raw_map_name) else {
        return RadarDebugSnapshot::no_map(
            tick,
            Some(gameplay_window),
            counts,
            format!("unsupported map key: {raw_map_name}"),
        );
    };
    let Some(map_image) = images.get(&map_key) else {
        return RadarDebugSnapshot::no_map(
            tick,
            Some(gameplay_window),
            counts,
            format!("missing image for map: {map_key}"),
        );
    };
    let Some(transform) = transforms.get(&map_key) else {
        return RadarDebugSnapshot::no_map(
            tick,
            Some(gameplay_window),
            counts,
            format!("missing transform for map: {map_key}"),
        );
    };

    let players = render_players(radar_players, transform, local_team);
    let shown_players = players.len() as u32;
    RadarDebugSnapshot {
        status: RadarDebugStatus::Ok,
        tick,
        message: "ok".to_string(),
        gameplay_window: Some(gameplay_window),
        map_key: Some(map_key),
        map_image: Some(map_image.clone()),
        counts: RadarDebugCounts {
            shown_players,
            ..counts
        },
        players,
    }
}

fn render_players(
    radar_players: &[RadarPlayerRecord],
    transform: &MapTransform,
    local_team: Option<i32>,
) -> Vec<RadarDebugPlayer> {
    let mut players = Vec::new();
    for player in radar_players {
        let [x, y] = world_to_radar(transform, player.origin);
        let relationship = if player.is_local {
            PlayerRelationship::SelfPlayer
        } else {
            match (Some(player.team_num), local_team) {
                (Some(team), Some(local)) if team == local => PlayerRelationship::Teammate,
                (Some(_), Some(_)) => PlayerRelationship::Enemy,
                _ => PlayerRelationship::Unknown,
            }
        };
        let display_name = if player.name.is_empty() {
            if player.steam_id == 0 {
                format!("pawn-{:X}", player.pawn)
            } else {
                format!("{}", player.steam_id)
            }
        } else {
            player.name.clone()
        };
        let stable_id = if player.steam_id == 0 {
            format!("pawn-{:X}", player.pawn)
        } else {
            format!("steam-{}", player.steam_id)
        };
        players.push(RadarDebugPlayer {
            id: stable_id,
            name: display_name,
            steam_id: Some(player.steam_id),
            pawn_runtime: Some(player.pawn as u64),
            x,
            y,
            health: Some(player.health),
            team_num: Some(player.team_num),
            life_state: Some(player.life_state),
            relationship,
            is_local: player.is_local,
            origin: Some(player.origin),
        });
    }
    players
}

fn connect_process() -> anyhow::Result<ActiveProcess> {
    let mut runtime = HostCs2Runtime::attach_host_cs2()?;
    let offsets = runtime.find_deadlocked_style_offsets()?;
    let global_vars_ptr = runtime.find_global_vars_ptr()?;
    Ok(ActiveProcess {
        runtime,
        offsets,
        global_vars_ptr,
    })
}

fn process_changed(active: &ActiveProcess) -> bool {
    let Ok(current) = resolve_host_process_by_name(PROCESS_NAME) else {
        return false;
    };
    current.pid != active.runtime.pid()
        || current.start_time_ticks != active.runtime.start_time_ticks()
}

fn assets_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../assets/radar")
}

fn build_compare_response(
    radar_snapshot: &RadarDebugSnapshot,
    snapshot: &Snapshot,
    radar_players: &[RadarPlayerRecord],
    counts: Option<LowLevelPlayerCounts>,
) -> RadarDebugCompareResponse {
    let mut by_pawn = BTreeMap::<u64, RadarDebugPlayerComparison>::new();

    for player in &snapshot.other_players {
        let pawn_runtime = player.id;
        by_pawn
            .entry(pawn_runtime)
            .or_insert(RadarDebugPlayerComparison {
                pawn_runtime,
                steam_id: None,
                snapshot: None,
                radar: None,
            })
            .snapshot = Some(SnapshotLayerPlayer {
            pawn_runtime,
            health: player.health,
            team_num: player.team_num,
            life_state: player.life_state,
            origin: player.origin,
        });
    }

    let rendered_by_pawn = radar_snapshot
        .players
        .iter()
        .filter_map(|player| player.pawn_runtime.map(|pawn| (pawn, (player.x, player.y))));

    let rendered_lookup = rendered_by_pawn.collect::<BTreeMap<_, _>>();

    for player in &radar_snapshot.players {
        let Some(pawn_runtime) = player.pawn_runtime else {
            continue;
        };
        let (radar_x, radar_y) = rendered_lookup
            .get(&pawn_runtime)
            .copied()
            .unwrap_or((f32::NAN, f32::NAN));
        let entry = by_pawn
            .entry(pawn_runtime)
            .or_insert(RadarDebugPlayerComparison {
                pawn_runtime,
                steam_id: player.steam_id,
                snapshot: None,
                radar: None,
            });
        entry.steam_id = player.steam_id;
        entry.radar = Some(RadarLayerPlayer {
            pawn_runtime,
            steam_id: player.steam_id,
            health: player.health,
            team_num: player.team_num,
            life_state: player.life_state,
            origin: player.origin.unwrap_or([f32::NAN; 3]),
            radar_x,
            radar_y,
        });
    }

    RadarDebugCompareResponse {
        tick: radar_snapshot.tick,
        map_key: radar_snapshot.map_key.clone(),
        map_image: radar_snapshot.map_image.clone(),
        gameplay_window: radar_snapshot.gameplay_window.clone(),
        counts: RadarDebugCompareCounts {
            low_level_server_players: counts.map(|value| value.server_players),
            low_level_other_players: counts.map(|value| value.other_players),
            low_level_resolved_other_players: counts.map(|value| value.resolved_other_players),
            snapshot_other_players: snapshot.other_players.len(),
            radar_record_players: radar_players.len(),
            rendered_players: radar_snapshot.players.len(),
        },
        comparisons: by_pawn.into_values().collect(),
    }
}

#[cfg(test)]
mod tests {}
