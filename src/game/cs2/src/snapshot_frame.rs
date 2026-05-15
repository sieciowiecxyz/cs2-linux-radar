use runtime_types::Snapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotPhase {
    Unknown,
    Menu,
    InMapNotPlayable,
    ReadyForBot,
}

impl SnapshotPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            SnapshotPhase::Unknown => "unknown",
            SnapshotPhase::Menu => "menu",
            SnapshotPhase::InMapNotPlayable => "in_map_not_playable",
            SnapshotPhase::ReadyForBot => "ready_for_bot",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotState {
    pub phase: SnapshotPhase,
    pub is_in_menu: bool,
    pub is_in_map: bool,
    pub is_alive: bool,
    pub is_ready_for_bot: bool,
}

pub fn classify_snapshot(snapshot: &Snapshot) -> SnapshotState {
    let is_in_menu = snapshot
        .map_name
        .as_deref()
        .is_none_or(|map| matches!(map.trim(), "" | "<empty>" | "maps/<empty>.vpk"));
    let has_real_map = !is_in_menu;
    let has_view_matrix = snapshot.view_matrix.is_some_and(|matrix| {
        matrix
            .iter()
            .any(|value| value.is_finite() && value.abs() > f32::EPSILON)
    });
    let has_origin = snapshot
        .local_player_state
        .as_ref()
        .and_then(|local| local.origin)
        .is_some_and(|origin| {
            origin.into_iter().all(f32::is_finite)
                && origin.iter().any(|value| value.abs() > f32::EPSILON)
        });
    let is_in_map = has_real_map && has_view_matrix && has_origin;
    let is_alive = snapshot.local_player_state.as_ref().is_some_and(|local| {
        matches!(local.health, Some(1..=100))
            && local.life_state == Some(256)
            && matches!(local.team_num, Some(2 | 3))
    });
    let is_ready_for_bot = is_in_map && is_alive;
    let phase = if is_in_menu {
        SnapshotPhase::Menu
    } else if is_ready_for_bot {
        SnapshotPhase::ReadyForBot
    } else if is_in_map {
        SnapshotPhase::InMapNotPlayable
    } else {
        SnapshotPhase::Unknown
    };

    SnapshotState {
        phase,
        is_in_menu,
        is_in_map,
        is_alive,
        is_ready_for_bot,
    }
}

pub fn derive_alive(snapshot: &Snapshot) -> bool {
    classify_snapshot(snapshot).is_alive
}

pub fn is_in_menu(snapshot: &Snapshot) -> bool {
    classify_snapshot(snapshot).is_in_menu
}

pub fn is_in_map(snapshot: &Snapshot) -> bool {
    classify_snapshot(snapshot).is_in_map
}

pub fn is_ready_for_bot(snapshot: &Snapshot) -> bool {
    classify_snapshot(snapshot).is_ready_for_bot
}

pub fn is_gameplay_ready(snapshot: &Snapshot) -> bool {
    is_ready_for_bot(snapshot)
}

#[cfg(test)]
mod tests {
    use runtime_types::{LocalPlayerState, Snapshot};

    use super::{
        SnapshotPhase, classify_snapshot, derive_alive, is_in_map, is_in_menu, is_ready_for_bot,
    };

    fn snapshot(
        map_name: Option<&str>,
        view_matrix: Option<[f32; 16]>,
        local_player_state: Option<LocalPlayerState>,
    ) -> Snapshot {
        Snapshot {
            map_name: map_name.map(str::to_owned),
            view_matrix,
            local_player_state,
            ..Snapshot::default()
        }
    }

    fn view_matrix() -> [f32; 16] {
        let mut matrix = [0.0; 16];
        matrix[0] = 1.0;
        matrix
    }

    #[test]
    fn menu_snapshot_is_detected() {
        let state = classify_snapshot(&snapshot(
            Some("<empty>"),
            Some(view_matrix()),
            Some(LocalPlayerState {
                health: Some(0),
                life_state: Some(258),
                team_num: Some(0),
                origin: Some([0.0, 0.0, 0.0]),
                ..LocalPlayerState::default()
            }),
        ));

        assert_eq!(state.phase, SnapshotPhase::Menu);
        assert!(state.is_in_menu);
        assert!(!state.is_in_map);
        assert!(!state.is_alive);
        assert!(!state.is_ready_for_bot);
    }

    #[test]
    fn search_or_spectator_state_is_in_map_but_not_playable() {
        let state = classify_snapshot(&snapshot(
            Some("de_cache"),
            Some(view_matrix()),
            Some(LocalPlayerState {
                health: Some(0),
                life_state: Some(258),
                team_num: Some(0),
                origin: Some([2456.58, 98.31, 1665.44]),
                ..LocalPlayerState::default()
            }),
        ));

        assert_eq!(state.phase, SnapshotPhase::InMapNotPlayable);
        assert!(!state.is_in_menu);
        assert!(state.is_in_map);
        assert!(!state.is_alive);
        assert!(!state.is_ready_for_bot);
    }

    #[test]
    fn ct_snapshot_is_ready_for_bot() {
        let snapshot = snapshot(
            Some("de_cache"),
            Some(view_matrix()),
            Some(LocalPlayerState {
                health: Some(100),
                life_state: Some(256),
                team_num: Some(3),
                origin: Some([689.50, 1060.50, 1700.03]),
                ..LocalPlayerState::default()
            }),
        );

        let state = classify_snapshot(&snapshot);
        assert_eq!(state.phase, SnapshotPhase::ReadyForBot);
        assert!(is_in_map(&snapshot));
        assert!(is_ready_for_bot(&snapshot));
        assert!(derive_alive(&snapshot));
    }

    #[test]
    fn t_snapshot_is_ready_for_bot() {
        let snapshot = snapshot(
            Some("de_cache"),
            Some(view_matrix()),
            Some(LocalPlayerState {
                health: Some(100),
                life_state: Some(256),
                team_num: Some(2),
                origin: Some([2441.50, 214.99, 1740.03]),
                ..LocalPlayerState::default()
            }),
        );

        let state = classify_snapshot(&snapshot);
        assert_eq!(state.phase, SnapshotPhase::ReadyForBot);
        assert!(state.is_alive);
        assert!(state.is_ready_for_bot);
    }

    #[test]
    fn dead_or_spectator_state_is_not_alive_even_on_real_map() {
        let snapshot = snapshot(
            Some("de_cache"),
            Some(view_matrix()),
            Some(LocalPlayerState {
                health: Some(0),
                life_state: Some(258),
                team_num: Some(1),
                origin: Some([2833.08, -105.92, 1608.03]),
                ..LocalPlayerState::default()
            }),
        );

        assert!(!derive_alive(&snapshot));
        assert!(is_in_map(&snapshot));
        assert!(!is_ready_for_bot(&snapshot));
        assert!(!is_in_menu(&snapshot));
    }
}
