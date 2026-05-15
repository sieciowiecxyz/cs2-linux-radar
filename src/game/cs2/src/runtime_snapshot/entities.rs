use anyhow::Result;
use runtime_types::{BoundingBox, EntityState, LocalPlayerState};

use super::SnapshotOffsets;
use super::memory::{SnapshotMemory, read_vec2, read_vec3};
use super::view::world_to_screen;

const ENTITY_RECORD_STRIDE: usize = 0x70;
const ENTITY_LIST_CHUNK_PTR_STRIDE: usize = 0x8;
const ENTITY_LIST_CHUNK_MASK: usize = 0x1FF;
const ENTITY_HANDLE_INDEX_MASK: u32 = 0x7FFF;
const PLAYER_SLOT_COUNT: usize = 64;
const AIM_PUNCH_CACHE_MAX_SAMPLES: usize = 256;

pub(super) fn read_other_players<B: SnapshotMemory>(
    runtime: &mut B,
    offsets: &SnapshotOffsets,
    local_pawn_addr: usize,
    view_matrix: Option<&[f32; 16]>,
) -> Result<Vec<EntityState>> {
    let mut out = Vec::new();
    for slot in 0..PLAYER_SLOT_COUNT {
        let Some(controller_addr) = entity_addr_for_index(runtime, offsets.entity_root, slot)
        else {
            continue;
        };
        let Some(pawn_addr) = resolve_live_player_pawn_addr(
            runtime,
            offsets,
            slot,
            controller_addr,
            local_pawn_addr,
        )?
        else {
            continue;
        };
        if pawn_addr == local_pawn_addr {
            continue;
        }

        let Some(candidate) = read_live_player_candidate(runtime, offsets, pawn_addr)? else {
            continue;
        };

        let LivePlayerCandidate {
            health,
            team_num,
            life_state,
            gun_game_immunity,
            origin,
        } = candidate;

        let (bbox_2d, head_pos_2d) = match (view_matrix, origin) {
            (Some(matrix), Some(origin)) => {
                let head = [origin[0], origin[1], origin[2] + 64.0];
                let head_2d = world_to_screen(matrix, head);
                let feet_2d = world_to_screen(matrix, origin);
                let bbox = match (head_2d, feet_2d) {
                    (Some(head_xy), Some(feet_xy)) => {
                        let height = (feet_xy[1] - head_xy[1]).abs();
                        let width = height * 0.4;
                        Some(BoundingBox {
                            left: head_xy[0] - width * 0.5,
                            right: head_xy[0] + width * 0.5,
                            top: head_xy[1],
                            bottom: feet_xy[1],
                        })
                    }
                    _ => None,
                };
                (bbox, head_2d)
            }
            _ => (None, None),
        };

        out.push(EntityState {
            id: pawn_addr as u64,
            health,
            team_num,
            life_state,
            gun_game_immunity,
            origin,
            head_pos: origin.map(|value| [value[0], value[1], value[2] + 64.0]),
            bbox_2d,
            head_pos_2d,
        });
    }

    Ok(out)
}

#[derive(Debug, Clone, Copy)]
struct LivePlayerCandidate {
    health: Option<i32>,
    team_num: Option<i32>,
    life_state: Option<i32>,
    gun_game_immunity: Option<bool>,
    origin: Option<[f32; 3]>,
}

fn read_live_player_candidate<B: SnapshotMemory>(
    runtime: &mut B,
    offsets: &SnapshotOffsets,
    pawn_addr: usize,
) -> Result<Option<LivePlayerCandidate>> {
    let health = runtime.read_i32(pawn_addr + offsets.pawn_health).ok();
    let team_num = runtime
        .read_u8(pawn_addr + offsets.pawn_team)
        .ok()
        .map(i32::from);
    let life_state = runtime
        .read_u32(pawn_addr + offsets.pawn_life_state)
        .ok()
        .map(|value| value as i32);

    let gun_game_immunity = runtime
        .read_u8(pawn_addr + offsets.pawn_deathmatch_immunity)
        .ok()
        .map(|value| value != 0);

    let origin = read_entity_abs_origin(runtime, offsets, pawn_addr).ok();
    if !origin.is_some_and(player_origin_plausible) {
        return Ok(None);
    }

    Ok(Some(LivePlayerCandidate {
        health,
        team_num,
        life_state,
        gun_game_immunity,
        origin,
    }))
}

pub(super) fn read_local_player_state<B: SnapshotMemory>(
    runtime: &mut B,
    offsets: &SnapshotOffsets,
    local_pawn_addr: usize,
    local_controller_addr: usize,
) -> Result<LocalPlayerState> {
    let origin = read_entity_abs_origin(runtime, offsets, local_pawn_addr).ok();
    let view_offset = read_vec3(runtime, local_pawn_addr + offsets.pawn_view_offset).ok();
    let view_origin = match (origin, view_offset) {
        (Some(origin), Some(offset)) => Some([
            origin[0] + offset[0],
            origin[1] + offset[1],
            origin[2] + offset[2],
        ]),
        _ => None,
    };
    let persona_level = read_persona_level(runtime, offsets, local_controller_addr);

    let local = LocalPlayerState {
        score: read_controller_score(runtime, offsets, local_controller_addr),
        health: runtime.read_i32(local_pawn_addr + offsets.pawn_health).ok(),
        team_num: runtime
            .read_u8(local_pawn_addr + offsets.pawn_team)
            .ok()
            .map(i32::from),
        life_state: runtime
            .read_u32(local_pawn_addr + offsets.pawn_life_state)
            .ok()
            .map(|value| value as i32),
        m_h_player_pawn: local_controller_addr != 0,
        shots_fired: runtime
            .read_i32(local_pawn_addr + offsets.pawn_shots_fired)
            .ok(),
        eye_angles: read_vec2(runtime, local_pawn_addr + offsets.pawn_eye_angles).ok(),
        view_angles: read_vec2(runtime, local_pawn_addr + offsets.pawn_view_angles).ok(),
        origin,
        view_origin,
        aim_punch_angle: read_aim_punch_vec2(runtime, local_pawn_addr, offsets).ok(),
        persona_level,
    };

    Ok(local)
}

fn read_controller_score<B: SnapshotMemory>(
    runtime: &mut B,
    offsets: &SnapshotOffsets,
    local_controller_addr: usize,
) -> Option<i32> {
    if local_controller_addr == 0 {
        return None;
    }
    runtime
        .read_i32(local_controller_addr + offsets.controller_score)
        .ok()
}

fn read_persona_level<B: SnapshotMemory>(
    runtime: &mut B,
    offsets: &SnapshotOffsets,
    local_controller_addr: usize,
) -> Option<i32> {
    if local_controller_addr == 0 {
        return None;
    }
    let inventory_services = runtime
        .read_u64(local_controller_addr + offsets.controller_inventory_services)
        .ok()? as usize;
    if inventory_services == 0 {
        return None;
    }
    runtime
        .read_i32(inventory_services + offsets.inventory_persona_public_level)
        .ok()
}

fn resolve_live_player_pawn_addr<B: SnapshotMemory>(
    runtime: &mut B,
    offsets: &SnapshotOffsets,
    _slot: usize,
    controller_addr: usize,
    local_pawn_addr: usize,
) -> Result<Option<usize>> {
    let direct_pawn_handle = runtime
        .read_u32(controller_addr + offsets.controller_pawn)
        .ok()
        .filter(|value| *value != 0 && *value != u32::MAX);
    let direct_pawn_addr = direct_pawn_handle
        .and_then(|handle| entity_addr_for_handle(runtime, offsets.entity_root, handle))
        .filter(|addr| *addr != 0 && process_addr_plausible(*addr));

    if let Some(pawn_addr) = direct_pawn_addr {
        if pawn_addr != local_pawn_addr {
            let class_name = runtime.read_instance_class_name(pawn_addr).ok().flatten();
            if class_name
                .as_deref()
                .is_some_and(is_gameplay_player_pawn_class)
                && read_live_player_candidate(runtime, offsets, pawn_addr)?.is_some()
            {
                return Ok(Some(pawn_addr));
            }
        }
    }

    Ok(None)
}

fn is_gameplay_player_pawn_class(class_name: &str) -> bool {
    class_name.ends_with("C_CSPlayerPawn")
}

pub(super) fn read_entity_abs_origin<B: SnapshotMemory>(
    runtime: &mut B,
    offsets: &SnapshotOffsets,
    entity_addr: usize,
) -> Result<[f32; 3]> {
    let scene_node_addr = runtime.read_u64(entity_addr + offsets.pawn_game_scene_node)? as usize;
    anyhow::ensure!(scene_node_addr != 0, "entity has null scene node");
    read_vec3(runtime, scene_node_addr + offsets.game_scene_node_origin)
}

fn entity_addr_for_index<B: SnapshotMemory>(
    runtime: &mut B,
    entity_list_addr: usize,
    index: usize,
) -> Option<usize> {
    let chunk_ptr_addr = entity_list_addr + ((index >> 9) * ENTITY_LIST_CHUNK_PTR_STRIDE);
    let chunk_addr = match runtime.read_u64(chunk_ptr_addr) {
        Ok(0) | Err(_) => return None,
        Ok(value) => value as usize,
    };
    match runtime.read_u64(chunk_addr + ((index & ENTITY_LIST_CHUNK_MASK) * ENTITY_RECORD_STRIDE)) {
        Ok(0) | Err(_) => None,
        Ok(value) => Some(value as usize),
    }
}

pub(super) fn entity_addr_for_handle<B: SnapshotMemory>(
    runtime: &mut B,
    entity_list_addr: usize,
    handle: u32,
) -> Option<usize> {
    let index = entity_handle_index(handle) as usize;
    entity_addr_for_index(runtime, entity_list_addr, index)
}

fn process_addr_plausible(address: usize) -> bool {
    (0x1_0000_0000..0x0000_8000_0000_0000).contains(&address)
}

fn entity_handle_index(handle: u32) -> u32 {
    handle & ENTITY_HANDLE_INDEX_MASK
}

fn read_aim_punch_vec2<B: SnapshotMemory>(
    runtime: &mut B,
    pawn_addr: usize,
    offsets: &SnapshotOffsets,
) -> Result<[f32; 2]> {
    let aim_punch_services_addr =
        runtime.read_u64(pawn_addr + offsets.pawn_aim_punch_services)? as usize;
    if aim_punch_services_addr == 0 {
        return Ok([0.0, 0.0]);
    }

    let length = runtime.read_u64(aim_punch_services_addr + offsets.aim_punch_cache)? as usize;
    if length == 0 {
        return Ok([0.0, 0.0]);
    }
    let length = length.min(AIM_PUNCH_CACHE_MAX_SAMPLES);

    let data_addr =
        runtime.read_u64(aim_punch_services_addr + offsets.aim_punch_cache + 0x08)? as usize;
    if !process_addr_plausible(data_addr) {
        return Ok([0.0, 0.0]);
    }

    let Some(sample_offset) = length.saturating_sub(1).checked_mul(12) else {
        return Ok([0.0, 0.0]);
    };
    let Some(sample_addr) = data_addr.checked_add(sample_offset) else {
        return Ok([0.0, 0.0]);
    };
    if !process_addr_plausible(sample_addr) {
        return Ok([0.0, 0.0]);
    }

    Ok([
        runtime.read_f32(sample_addr)?,
        runtime.read_f32(sample_addr + 4)?,
    ])
}

pub(super) fn vec3_plausible(vec: [f32; 3]) -> bool {
    vec.into_iter()
        .all(|value| value.is_finite() && value.abs() < 100_000.0)
}

fn player_origin_plausible(vec: [f32; 3]) -> bool {
    vec3_plausible(vec) && vec.into_iter().any(|value| value.abs() > f32::EPSILON)
}
