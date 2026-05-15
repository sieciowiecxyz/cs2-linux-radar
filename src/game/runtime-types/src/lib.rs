#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoundingBox {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct LocalPlayerState {
    pub score: Option<i32>,
    pub health: Option<i32>,
    pub team_num: Option<i32>,
    pub life_state: Option<i32>,
    pub m_h_player_pawn: bool,
    pub shots_fired: Option<i32>,
    pub eye_angles: Option<[f32; 2]>,
    pub origin: Option<[f32; 3]>,
    pub view_origin: Option<[f32; 3]>,
    pub aim_punch_angle: Option<[f32; 2]>,
    pub view_angles: Option<[f32; 2]>,
    pub persona_level: Option<i32>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct EntityState {
    pub id: u64,
    pub health: Option<i32>,
    pub team_num: Option<i32>,
    pub life_state: Option<i32>,
    pub gun_game_immunity: Option<bool>,
    pub origin: Option<[f32; 3]>,
    pub head_pos: Option<[f32; 3]>,
    pub bbox_2d: Option<BoundingBox>,
    pub head_pos_2d: Option<[f32; 2]>,
}

impl EntityState {
    pub fn is_gungame_immune(&self) -> bool {
        matches!(self.gun_game_immunity, Some(true))
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Snapshot {
    pub map_name: Option<String>,
    pub game_time: Option<f32>,
    pub view_matrix: Option<[f32; 16]>,
    pub local_player_state: Option<LocalPlayerState>,
    pub other_players: Vec<EntityState>,
}

pub fn sanitize_semantic_team_num(team_num: Option<i32>) -> Option<i32> {
    match team_num {
        Some(team @ 0..=3) => Some(team),
        _ => None,
    }
}

pub fn sanitize_semantic_life_state(life_state: Option<i32>) -> Option<i32> {
    match life_state {
        Some(0 | 1 | 2 | 256 | 257 | 258) => life_state,
        _ => None,
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeFaultSubsystem {
    #[default]
    Aimer,
    Walkbot,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeFaultSeverity {
    #[default]
    Info,
    Warn,
    Degraded,
    Fatal,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeFaultKind {
    #[default]
    AimerFireStall,
    WalkbotPlanningFailure,
    WalkbotGuidanceSync,
    WalkbotStuckDetected,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct RuntimeFault {
    pub subsystem: RuntimeFaultSubsystem,
    pub severity: RuntimeFaultSeverity,
    pub kind: RuntimeFaultKind,
    pub target_id: Option<u64>,
    pub target_idx: Option<u16>,
    pub queue_len: Option<u16>,
    pub tracking_age_ms: Option<u64>,
    pub shots_fired: Option<i32>,
    pub aim_punch_mag_deg: Option<f32>,
    pub stall_ms: Option<u64>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveIntent {
    pub forward: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    pub sprint: bool,
    pub duck: bool,
    pub jump: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FireAction {
    #[default]
    None,
    Tap,
    SetHeld(bool),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StrafeDir {
    #[default]
    Left,
    Right,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CombatMoveOverlay {
    #[default]
    None,
    CounterStrafe {
        dir: StrafeDir,
        hold_ms: u16,
    },
    ADBait {
        dir: StrafeDir,
        hold_ms: u16,
    },
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct AimIntent {
    pub dx: i32,
    pub dy: i32,
    pub hold_crouch: bool,
    pub hold_shift: bool,
    pub fire: FireAction,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NavigationLookMode {
    #[default]
    Follow,
    Precision,
    Recovery,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NavigationLookSource {
    #[default]
    Lookahead,
    Waypoint,
    Recovery,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct NavigationLookHint {
    pub desired_heading_deg: f32,
    pub path_heading_deg: Option<f32>,
    pub lookahead_point_xy: Option<[f32; 2]>,
    pub active_waypoint_xy: Option<[f32; 2]>,
    pub mode: NavigationLookMode,
    pub source: NavigationLookSource,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CombatNavIntent {
    #[default]
    KeepRoute,
    SuspendRoute,
    SoftPauseForShot,
    HoldPosition,
}

#[cfg(test)]
mod tests {
    use super::{
        EntityState, RuntimeFault, RuntimeFaultKind, RuntimeFaultSeverity, RuntimeFaultSubsystem,
        sanitize_semantic_life_state, sanitize_semantic_team_num,
    };

    #[test]
    fn entity_is_not_immune_when_flag_missing() {
        let entity = EntityState::default();
        assert!(!entity.is_gungame_immune());
    }

    #[test]
    fn entity_is_not_immune_when_flag_false() {
        let entity = EntityState {
            gun_game_immunity: Some(false),
            ..EntityState::default()
        };
        assert!(!entity.is_gungame_immune());
    }

    #[test]
    fn entity_reports_gungame_immunity() {
        let entity = EntityState {
            gun_game_immunity: Some(true),
            ..EntityState::default()
        };
        assert!(entity.is_gungame_immune());
    }

    #[test]
    fn runtime_fault_round_trips_copy_defaults() {
        let fault = RuntimeFault {
            subsystem: RuntimeFaultSubsystem::Aimer,
            severity: RuntimeFaultSeverity::Degraded,
            kind: RuntimeFaultKind::AimerFireStall,
            target_id: Some(7),
            target_idx: None,
            queue_len: None,
            tracking_age_ms: Some(320),
            shots_fired: Some(0),
            aim_punch_mag_deg: Some(0.1),
            stall_ms: Some(240),
        };
        let copied = fault;

        assert_eq!(copied.subsystem, RuntimeFaultSubsystem::Aimer);
        assert_eq!(copied.severity, RuntimeFaultSeverity::Degraded);
        assert_eq!(copied.kind, RuntimeFaultKind::AimerFireStall);
        assert_eq!(copied.target_id, Some(7));
        assert_eq!(copied.stall_ms, Some(240));
    }

    #[test]
    fn semantic_team_num_drops_garbage_values() {
        assert_eq!(sanitize_semantic_team_num(Some(0)), Some(0));
        assert_eq!(sanitize_semantic_team_num(Some(2)), Some(2));
        assert_eq!(sanitize_semantic_team_num(Some(3)), Some(3));
        assert_eq!(sanitize_semantic_team_num(Some(100666923)), None);
        assert_eq!(sanitize_semantic_team_num(Some(-7)), None);
    }

    #[test]
    fn semantic_life_state_drops_garbage_values() {
        assert_eq!(sanitize_semantic_life_state(Some(0)), Some(0));
        assert_eq!(sanitize_semantic_life_state(Some(256)), Some(256));
        assert_eq!(sanitize_semantic_life_state(Some(258)), Some(258));
        assert_eq!(sanitize_semantic_life_state(Some(9152)), None);
        assert_eq!(sanitize_semantic_life_state(Some(-1)), None);
    }
}
