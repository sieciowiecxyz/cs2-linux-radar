use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use deadlocked_headless::{DeadlockedStyleOffsets, ModuleImage};
use memreader_client::{
    DEFAULT_TARGET_SLOT, MemReaderDevice, MemReaderModuleInfo, MemReaderTargetSession,
    TargetSelector,
};
use tracing::debug;

use crate::pattern::{find_matches, parse_pattern};
use crate::runtime_snapshot;
use crate::runtime_snapshot::{MemoryReader, SnapshotMemory};

use super::interfaces;
use super::process;

const MODULE_SCAN_CHUNK_BYTES: usize = 512 * 1024;
const GLOBAL_VARS_PATTERN: &str = "48 8D 05 ? ? ? ? 48 8B 00 8B 48 ? E9";

pub(super) struct MemReaderMemory {
    inner: MemReaderTargetSession,
}

impl MemReaderMemory {
    pub(super) fn read_c_string(&mut self, address: usize, max_len: usize) -> Result<String> {
        if max_len == 0 {
            bail!("read_c_string requires max_len > 0");
        }
        let bytes = self.inner.read_bytes(address, max_len as u32)?;
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(bytes.len());
        Ok(String::from_utf8_lossy(&bytes[..end]).to_string())
    }
}

impl MemoryReader for MemReaderMemory {
    fn read_exact_at(&mut self, address: usize, buf: &mut [u8]) -> Result<()> {
        self.inner.read_exact_at(address, buf)
    }
}

pub struct HostCs2Runtime {
    pid: u32,
    start_time_ticks: u64,
    modules: Vec<MemReaderModuleInfo>,
    memory: MemReaderMemory,
}

impl deadlocked_headless::HeadlessRuntime for HostCs2Runtime {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn module_image(&self, suffix: &str) -> Result<ModuleImage> {
        HostCs2Runtime::module_image(self, suffix)
    }

    fn read_module_file(&self, suffix: &str) -> Result<Vec<u8>> {
        HostCs2Runtime::read_module_file(self, suffix)
    }

    fn scan_module_memory_pattern(
        &mut self,
        module_suffix: &str,
        pattern: &str,
    ) -> Result<Option<usize>> {
        HostCs2Runtime::scan_module_memory_pattern(self, module_suffix, pattern)
    }

    fn read_relative_address(
        &mut self,
        instruction: usize,
        displacement_offset: usize,
        instruction_size: usize,
    ) -> Result<usize> {
        HostCs2Runtime::read_relative_address(
            self,
            instruction,
            displacement_offset,
            instruction_size,
        )
    }

    fn read_exact_at(&mut self, address: usize, buf: &mut [u8]) -> Result<()> {
        self.memory.inner.read_exact_at(address, buf)
    }
}

impl MemoryReader for HostCs2Runtime {
    fn read_exact_at(&mut self, address: usize, buf: &mut [u8]) -> Result<()> {
        self.memory.inner.read_exact_at(address, buf)
    }
}

impl SnapshotMemory for HostCs2Runtime {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn mapped_modules(&self) -> Vec<MemReaderModuleInfo> {
        self.modules.clone()
    }

    fn read_module_file(&self, suffix: &str) -> Result<Vec<u8>> {
        HostCs2Runtime::read_module_file(self, suffix)
    }

    fn resolve_interface_instance(
        &mut self,
        module_suffix: &str,
        interface_name: &str,
    ) -> Result<Option<usize>> {
        let module = self.module_image(module_suffix)?;
        let file_bytes = self.read_module_file(module_suffix)?;
        interfaces::resolve_interface_instance(
            &mut self.memory,
            &module,
            &file_bytes,
            interface_name,
        )
    }

    fn read_instance_class_name(&mut self, instance_addr: usize) -> Result<Option<String>> {
        interfaces::read_instance_class_name(&mut self.memory, instance_addr)
    }
}

impl HostCs2Runtime {
    pub fn attach_host_cs2() -> Result<Self> {
        let pid = process::resolve_pid(None)?;
        let start_time_ticks = process::inspect_process_start_time(pid)?;
        Self::attach_host_process(DEFAULT_TARGET_SLOT, pid, start_time_ticks)
    }

    pub fn attach_host_process(slot: u32, pid: u32, start_time_ticks: u64) -> Result<Self> {
        let selector = TargetSelector::host_pid(pid, start_time_ticks);
        let modules = MemReaderDevice::open()?
            .list_modules(selector)
            .context("list host process modules via memreader")?;
        let session = MemReaderTargetSession::open_host_process(slot, pid, start_time_ticks)?;
        debug!(
            pid,
            start_time_ticks,
            module_count = modules.len(),
            "attached host cs2 runtime via memreader"
        );
        Ok(Self {
            pid,
            start_time_ticks,
            modules,
            memory: MemReaderMemory { inner: session },
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn start_time_ticks(&self) -> u64 {
        self.start_time_ticks
    }

    fn module_image(&self, suffix: &str) -> Result<ModuleImage> {
        interfaces::module_image(&self.modules, suffix)
            .with_context(|| format!("missing module image for `{suffix}`"))
    }

    fn read_module_file(&self, suffix: &str) -> Result<Vec<u8>> {
        let image = self.module_image(suffix)?;
        fs::read(&image.path)
            .or_else(|_| read_module_mirror(&image.path))
            .or_else(|_| {
                fs::read(image.path.replacen(
                    "/srv/mfc/cs2-library",
                    "/var/lib/incus/storage-pools/default/custom/default_mfc-cs2-library",
                    1,
                ))
            })
            .with_context(|| format!("failed to read module file {}", image.path))
    }

    pub fn find_deadlocked_style_offsets(&mut self) -> Result<DeadlockedStyleOffsets> {
        deadlocked_headless::find_deadlocked_style_offsets(self)
    }

    pub fn find_global_vars_ptr(&mut self) -> Result<usize> {
        let instruction = self
            .scan_module_memory_pattern("libclient.so", GLOBAL_VARS_PATTERN)?
            .context("missing global vars pattern")?;
        self.read_relative_address(instruction, 0x03, 0x07)
    }

    fn scan_module_memory_pattern(
        &mut self,
        module_suffix: &str,
        pattern: &str,
    ) -> Result<Option<usize>> {
        let tokens = parse_pattern(pattern)?;
        let pattern_len = pattern
            .split_whitespace()
            .filter(|token| !token.is_empty())
            .count()
            .max(1);

        let mut segments = self
            .modules
            .iter()
            .filter(|module| module.path.ends_with(module_suffix) && module.perms.starts_with('r'))
            .cloned()
            .collect::<Vec<_>>();
        segments.sort_unstable_by_key(|module| module.base);

        for segment in segments {
            let mut offset = 0usize;
            while segment.base + offset < segment.end {
                let remaining = segment.end - (segment.base + offset);
                let chunk_len = remaining.min(MODULE_SCAN_CHUNK_BYTES);
                let bytes = self
                    .memory
                    .inner
                    .read_bytes(segment.base + offset, chunk_len as u32)?;
                if let Some(found) = find_matches(&bytes, &tokens, 1).into_iter().next() {
                    return Ok(Some(segment.base + offset + found));
                }
                if remaining <= chunk_len {
                    break;
                }
                offset += chunk_len.saturating_sub(pattern_len.saturating_sub(1));
            }
        }

        Ok(None)
    }

    fn read_relative_address(
        &mut self,
        instruction: usize,
        displacement_offset: usize,
        instruction_size: usize,
    ) -> Result<usize> {
        let mut disp_buf = [0; 4];
        self.memory
            .read_exact_at(instruction + displacement_offset, &mut disp_buf)?;
        let disp = i32::from_le_bytes(disp_buf) as i64;
        Ok(((instruction + instruction_size) as i64 + disp) as usize)
    }

    pub(super) fn read_snapshot(
        &mut self,
        offsets: &runtime_snapshot::SnapshotOffsets,
    ) -> Result<runtime_types::Snapshot> {
        runtime_snapshot::read_snapshot(self, offsets)
    }

    pub fn read_snapshot_with_deadlocked_offsets(
        &mut self,
        offsets: &DeadlockedStyleOffsets,
        global_vars_ptr: usize,
    ) -> Result<runtime_types::Snapshot> {
        self.read_snapshot(&runtime_snapshot::SnapshotOffsets {
            global_vars_ptr,
            entity_root: offsets.entity_root,
            local_player_controller_ptr: offsets.local_player_controller,
            controller_pawn: offsets.controller_pawn as usize,
            controller_score: offsets.controller_score as usize,
            controller_inventory_services: offsets.controller_inventory_services as usize,
            inventory_persona_public_level: offsets.inventory_persona_public_level as usize,
            pawn_health: offsets.pawn_health as usize,
            pawn_team: offsets.pawn_team as usize,
            pawn_life_state: offsets.pawn_life_state as usize,
            pawn_game_scene_node: offsets.pawn_game_scene_node as usize,
            pawn_view_offset: offsets.pawn_eye_offset as usize,
            pawn_eye_angles: offsets.pawn_eye_angles as usize,
            pawn_view_angles: offsets.pawn_view_angles as usize,
            pawn_shots_fired: offsets.pawn_shots_fired as usize,
            pawn_aim_punch_services: offsets.pawn_aim_punch_services as usize,
            aim_punch_cache: offsets.aim_punch_cache as usize,
            pawn_deathmatch_immunity: offsets.pawn_deathmatch_immunity as usize,
            game_scene_node_origin: offsets.game_scene_node_origin as usize,
        })
    }

    pub fn read_local_player_record(
        &mut self,
        offsets: &DeadlockedStyleOffsets,
    ) -> Result<Option<deadlocked_headless::LocalPlayerRecord>> {
        deadlocked_headless::read_local_player_record(self, offsets)
    }

    pub fn read_radar_player_records(
        &mut self,
        offsets: &DeadlockedStyleOffsets,
    ) -> Result<Vec<deadlocked_headless::RadarPlayerRecord>> {
        deadlocked_headless::read_radar_player_records(self, offsets)
    }

    pub fn resolve_entity_index(
        &mut self,
        offsets: &DeadlockedStyleOffsets,
        index: u32,
    ) -> Result<Option<usize>> {
        let bucket_index = (index >> 9) as usize;
        let index_in_bucket = (index & 0x1FF) as usize;
        let bucket_ptr = self.read_u64(
            offsets
                .entity_root
                .checked_add(
                    bucket_index
                        .checked_mul(8)
                        .context("entity bucket overflow")?,
                )
                .context("entity root overflow")?,
        )? as usize;
        if bucket_ptr == 0 {
            return Ok(None);
        }
        let entity = self.read_u64(
            bucket_ptr
                .checked_add(
                    index_in_bucket
                        .checked_mul(offsets.entity_identity_size)
                        .context("entity identity overflow")?,
                )
                .context("entity bucket pointer overflow")?,
        )? as usize;
        Ok((entity != 0).then_some(entity))
    }

    pub fn read_bone_position(
        &mut self,
        offsets: &DeadlockedStyleOffsets,
        pawn: usize,
        bone_index: u64,
    ) -> Result<Option<[f32; 3]>> {
        let game_scene_node = self.read_u64(
            pawn.checked_add(offsets.pawn_game_scene_node as usize)
                .context("pawn game scene node overflow")?,
        )? as usize;
        if game_scene_node == 0 {
            return Ok(None);
        }
        let bone_data = self.read_u64(
            game_scene_node
                .checked_add(offsets.game_scene_node_model_state as usize)
                .and_then(|value| value.checked_add(offsets.model_state_skeleton_instance as usize))
                .context("bone data pointer overflow")?,
        )? as usize;
        if bone_data == 0 {
            return Ok(None);
        }
        let bone_addr = bone_data
            .checked_add(
                (bone_index as usize)
                    .checked_mul(32)
                    .context("bone index overflow")?,
            )
            .context("bone address overflow")?;
        Ok(Some([
            self.read_f32(bone_addr)?,
            self.read_f32(bone_addr + 4)?,
            self.read_f32(bone_addr + 8)?,
        ]))
    }
}

fn read_module_mirror(module_path: &str) -> std::io::Result<Vec<u8>> {
    let mut roots = Vec::new();
    if let Some(root) = std::env::var_os("MFC_CS2_MODULE_MIRROR") {
        roots.push(PathBuf::from(root));
    }
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(Path::new(&home).join(".local/share/Steam"));
    }

    let steam_relative = module_path.strip_prefix("/srv/mfc/cs2-library/");
    let absolute_relative = module_path.strip_prefix('/').unwrap_or(module_path);
    for root in roots {
        if let Some(relative) = steam_relative {
            if let Ok(bytes) = fs::read(root.join(relative)) {
                return Ok(bytes);
            }
        }
        if let Ok(bytes) = fs::read(root.join(absolute_relative)) {
            return Ok(bytes);
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "module mirror not found",
    ))
}
