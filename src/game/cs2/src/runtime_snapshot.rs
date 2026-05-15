mod entities;
mod memory;
mod network;
mod view;

use anyhow::Result;
use runtime_types::Snapshot;

use self::entities::{entity_addr_for_handle, read_local_player_state, read_other_players};
pub(crate) use self::memory::{MemoryReader, SnapshotMemory};
use self::network::read_map_name;
use self::view::read_view_matrix;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SnapshotOffsets {
    pub(crate) global_vars_ptr: usize,
    pub(crate) entity_root: usize,
    pub(crate) local_player_controller_ptr: usize,
    pub(crate) controller_pawn: usize,
    pub(crate) controller_score: usize,
    pub(crate) controller_inventory_services: usize,
    pub(crate) inventory_persona_public_level: usize,
    pub(crate) pawn_health: usize,
    pub(crate) pawn_team: usize,
    pub(crate) pawn_life_state: usize,
    pub(crate) pawn_game_scene_node: usize,
    pub(crate) pawn_view_offset: usize,
    pub(crate) pawn_eye_angles: usize,
    pub(crate) pawn_view_angles: usize,
    pub(crate) pawn_shots_fired: usize,
    pub(crate) pawn_aim_punch_services: usize,
    pub(crate) aim_punch_cache: usize,
    pub(crate) pawn_deathmatch_immunity: usize,
    pub(crate) game_scene_node_origin: usize,
}

pub(crate) fn read_snapshot<B: SnapshotMemory>(
    runtime: &mut B,
    offsets: &SnapshotOffsets,
) -> Result<Snapshot> {
    let mut snapshot = Snapshot {
        map_name: read_map_name(runtime),
        game_time: read_game_time(runtime, offsets),
        ..Default::default()
    };

    let local_controller_addr = runtime
        .read_u64(offsets.local_player_controller_ptr)
        .unwrap_or_default() as usize;
    let local_pawn_handle = if local_controller_addr != 0 {
        runtime
            .read_u32(local_controller_addr + offsets.controller_pawn)
            .unwrap_or_default()
    } else {
        0
    };
    let local_pawn_addr = if local_pawn_handle != 0 && local_pawn_handle != u32::MAX {
        entity_addr_for_handle(runtime, offsets.entity_root, local_pawn_handle).unwrap_or_default()
    } else {
        0
    };
    if local_pawn_addr == 0 {
        return Ok(snapshot);
    }

    snapshot.view_matrix = read_view_matrix(runtime, offsets, local_pawn_addr).ok();
    snapshot.local_player_state = Some(read_local_player_state(
        runtime,
        offsets,
        local_pawn_addr,
        local_controller_addr,
    )?);
    snapshot.other_players = read_other_players(
        runtime,
        offsets,
        local_pawn_addr,
        snapshot.view_matrix.as_ref(),
    )?;
    Ok(snapshot)
}

fn read_game_time<B: SnapshotMemory>(runtime: &mut B, offsets: &SnapshotOffsets) -> Option<f32> {
    let global_vars = runtime.read_u64(offsets.global_vars_ptr).ok()? as usize;
    (global_vars != 0)
        .then(|| runtime.read_f32(global_vars + 0x30).ok())
        .flatten()
}
