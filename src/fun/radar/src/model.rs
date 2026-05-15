use serde::Serialize;

pub type GameplayWindow = String;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RadarDebugStatus {
    Booting,
    Ok,
    NoMap,
    ReaderUnavailable,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerRelationship {
    SelfPlayer,
    Enemy,
    Teammate,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
pub struct RadarDebugPlayer {
    pub id: String,
    pub name: String,
    pub steam_id: Option<u64>,
    pub pawn_runtime: Option<u64>,
    pub x: f32,
    pub y: f32,
    pub health: Option<i32>,
    pub team_num: Option<i32>,
    pub life_state: Option<i32>,
    pub relationship: PlayerRelationship,
    pub is_local: bool,
    pub origin: Option<[f32; 3]>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct RadarDebugCounts {
    pub low_level_server_players: Option<u32>,
    pub low_level_other_players: Option<u32>,
    pub low_level_resolved_other_players: Option<u32>,
    pub snapshot_other_players: Option<u32>,
    pub shown_players: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct RadarDebugSnapshot {
    pub status: RadarDebugStatus,
    pub tick: u64,
    pub message: String,
    pub gameplay_window: Option<GameplayWindow>,
    pub map_key: Option<String>,
    pub map_image: Option<String>,
    pub counts: RadarDebugCounts,
    pub players: Vec<RadarDebugPlayer>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RadarDebugCompareCounts {
    pub low_level_server_players: Option<u32>,
    pub low_level_other_players: Option<u32>,
    pub low_level_resolved_other_players: Option<u32>,
    pub snapshot_other_players: usize,
    pub radar_record_players: usize,
    pub rendered_players: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct SnapshotLayerPlayer {
    pub pawn_runtime: u64,
    pub health: Option<i32>,
    pub team_num: Option<i32>,
    pub life_state: Option<i32>,
    pub origin: Option<[f32; 3]>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RadarLayerPlayer {
    pub pawn_runtime: u64,
    pub steam_id: Option<u64>,
    pub health: Option<i32>,
    pub team_num: Option<i32>,
    pub life_state: Option<i32>,
    pub origin: [f32; 3],
    pub radar_x: f32,
    pub radar_y: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct RadarDebugPlayerComparison {
    pub pawn_runtime: u64,
    pub steam_id: Option<u64>,
    pub snapshot: Option<SnapshotLayerPlayer>,
    pub radar: Option<RadarLayerPlayer>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RadarDebugCompareResponse {
    pub tick: u64,
    pub map_key: Option<String>,
    pub map_image: Option<String>,
    pub gameplay_window: Option<GameplayWindow>,
    pub counts: RadarDebugCompareCounts,
    pub comparisons: Vec<RadarDebugPlayerComparison>,
}

impl RadarDebugSnapshot {
    pub fn booting() -> Self {
        Self {
            status: RadarDebugStatus::Booting,
            tick: 0,
            message: "booting".to_string(),
            gameplay_window: None,
            map_key: None,
            map_image: None,
            counts: RadarDebugCounts::default(),
            players: Vec::new(),
        }
    }

    pub fn reader_unavailable(message: impl Into<String>) -> Self {
        Self {
            status: RadarDebugStatus::ReaderUnavailable,
            tick: 0,
            message: message.into(),
            gameplay_window: None,
            map_key: None,
            map_image: None,
            counts: RadarDebugCounts::default(),
            players: Vec::new(),
        }
    }

    pub fn no_map(
        tick: u64,
        gameplay_window: Option<GameplayWindow>,
        counts: RadarDebugCounts,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status: RadarDebugStatus::NoMap,
            tick,
            message: message.into(),
            gameplay_window,
            map_key: None,
            map_image: None,
            counts,
            players: Vec::new(),
        }
    }
}
