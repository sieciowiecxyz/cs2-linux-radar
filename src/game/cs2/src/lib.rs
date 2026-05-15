#![forbid(unsafe_code)]

mod attach;
mod elf;
mod pattern;
mod runtime_snapshot;
mod snapshot_frame;

pub use attach::{
    DeadlockedStyleOffsets, HostCs2Runtime, LocalPlayerRecord, PROCESS_NAME, RadarPlayerRecord,
    SnapshotSource, connect, resolve_pid,
};
pub use snapshot_frame::{
    SnapshotPhase, SnapshotState, classify_snapshot, derive_alive, is_gameplay_ready, is_in_map,
    is_in_menu, is_ready_for_bot,
};
