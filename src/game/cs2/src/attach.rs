mod host_runtime;
mod interfaces;
mod process;

use crate::runtime_snapshot;
use anyhow::Result;
pub use deadlocked_headless::{DeadlockedStyleOffsets, LocalPlayerRecord, RadarPlayerRecord};
pub use host_runtime::HostCs2Runtime;
use memreader_client::DEFAULT_TARGET_SLOT;
use runtime_types::Snapshot;

pub const PROCESS_NAME: &str = process::PROCESS_NAME;

pub trait SnapshotSource: Send {
    fn read_snapshot(&mut self) -> Snapshot;
}

impl SnapshotSource for Box<dyn SnapshotSource> {
    fn read_snapshot(&mut self) -> Snapshot {
        (**self).read_snapshot()
    }
}

struct Cs2SnapshotReader {
    runtime: HostCs2Runtime,
    offsets: DeadlockedStyleOffsets,
    global_vars_ptr: usize,
}

impl SnapshotSource for Cs2SnapshotReader {
    fn read_snapshot(&mut self) -> Snapshot {
        self.runtime
            .read_snapshot(&self.snapshot_offsets())
            .unwrap_or_default()
    }
}

impl Cs2SnapshotReader {
    fn snapshot_offsets(&self) -> runtime_snapshot::SnapshotOffsets {
        runtime_snapshot::SnapshotOffsets {
            global_vars_ptr: self.global_vars_ptr,
            entity_root: self.offsets.entity_root,
            local_player_controller_ptr: self.offsets.local_player_controller,
            controller_pawn: self.offsets.controller_pawn as usize,
            controller_score: self.offsets.controller_score as usize,
            controller_inventory_services: self.offsets.controller_inventory_services as usize,
            inventory_persona_public_level: self.offsets.inventory_persona_public_level as usize,
            pawn_health: self.offsets.pawn_health as usize,
            pawn_team: self.offsets.pawn_team as usize,
            pawn_life_state: self.offsets.pawn_life_state as usize,
            pawn_game_scene_node: self.offsets.pawn_game_scene_node as usize,
            pawn_view_offset: self.offsets.pawn_eye_offset as usize,
            pawn_eye_angles: self.offsets.pawn_eye_angles as usize,
            pawn_view_angles: self.offsets.pawn_view_angles as usize,
            pawn_shots_fired: self.offsets.pawn_shots_fired as usize,
            pawn_aim_punch_services: self.offsets.pawn_aim_punch_services as usize,
            aim_punch_cache: self.offsets.aim_punch_cache as usize,
            pawn_deathmatch_immunity: self.offsets.pawn_deathmatch_immunity as usize,
            game_scene_node_origin: self.offsets.game_scene_node_origin as usize,
        }
    }
}

pub fn connect(pid: u32) -> Result<impl SnapshotSource + 'static> {
    process::ensure_process_name(pid, PROCESS_NAME)?;
    let start_time_ticks = process::inspect_process_start_time(pid)?;
    let mut runtime =
        HostCs2Runtime::attach_host_process(DEFAULT_TARGET_SLOT, pid, start_time_ticks)?;
    let offsets = runtime.find_deadlocked_style_offsets()?;
    let global_vars_ptr = runtime.find_global_vars_ptr()?;
    Ok(Cs2SnapshotReader {
        runtime,
        offsets,
        global_vars_ptr,
    })
}

pub fn resolve_pid(requested_pid: Option<u32>) -> Result<u32> {
    process::resolve_pid(requested_pid)
}
