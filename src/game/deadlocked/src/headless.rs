use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};

use crate::elf::{file_offset_for_virtual_addr, find_symbol_virtual_addr};
use crate::pattern::{find_matches, parse_pattern};

const CREATE_INTERFACE_SYMBOL: &str = "CreateInterface";
const INTERFACE_LIST_PATTERN: &str = "48 8B 1D ? ? ? ? 48 85 DB 74 ?";
const CREATE_FN_RET_PATTERN: &[u8] = &[0x48, 0x8D, 0x05];
const SCHEMA_SYSTEM_PATTERN: &str =
    "48 8D 3D ? ? ? ? E8 ? ? ? ? 48 8B BD ? ? ? ? 31 F6 E8 ? ? ? ? E9";

fn checked_addr(base: usize, offset: usize, context: &'static str) -> Result<usize> {
    base.checked_add(offset)
        .with_context(|| format!("{context}: address overflow base={base:#x} offset={offset:#x}"))
}

fn checked_mul(index: usize, stride: usize, context: &'static str) -> Result<usize> {
    index.checked_mul(stride).with_context(|| {
        format!("{context}: multiplication overflow index={index} stride={stride}")
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleImage {
    pub path: String,
    pub image_base: usize,
    pub image_end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceEntry {
    pub name: String,
    pub create_fn: usize,
}

pub trait HeadlessRuntime {
    fn pid(&self) -> u32;
    fn module_image(&self, suffix: &str) -> Result<ModuleImage>;
    fn read_module_file(&self, suffix: &str) -> Result<Vec<u8>>;
    fn scan_module_memory_pattern(
        &mut self,
        module_suffix: &str,
        pattern: &str,
    ) -> Result<Option<usize>>;
    fn read_relative_address(
        &mut self,
        instruction: usize,
        displacement_offset: usize,
        instruction_size: usize,
    ) -> Result<usize>;
    fn read_exact_at(&mut self, address: usize, buf: &mut [u8]) -> Result<()>;

    fn read_u64(&mut self, address: usize) -> Result<u64> {
        let mut buf = [0; 8];
        self.read_exact_at(address, &mut buf)?;
        Ok(u64::from_le_bytes(buf))
    }

    fn read_i32(&mut self, address: usize) -> Result<i32> {
        let mut buf = [0; 4];
        self.read_exact_at(address, &mut buf)?;
        Ok(i32::from_le_bytes(buf))
    }

    fn read_i16(&mut self, address: usize) -> Result<i16> {
        let mut buf = [0; 2];
        self.read_exact_at(address, &mut buf)?;
        Ok(i16::from_le_bytes(buf))
    }

    fn read_u32(&mut self, address: usize) -> Result<u32> {
        let mut buf = [0; 4];
        self.read_exact_at(address, &mut buf)?;
        Ok(u32::from_le_bytes(buf))
    }

    fn read_u8(&mut self, address: usize) -> Result<u8> {
        let mut buf = [0; 1];
        self.read_exact_at(address, &mut buf)?;
        Ok(buf[0])
    }

    fn read_f32(&mut self, address: usize) -> Result<f32> {
        let mut buf = [0; 4];
        self.read_exact_at(address, &mut buf)?;
        Ok(f32::from_le_bytes(buf))
    }

    fn read_bytes(&mut self, address: usize, count: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0; count];
        self.read_exact_at(address, &mut buf)?;
        Ok(buf)
    }

    fn read_c_string(&mut self, address: usize, max_len: usize) -> Result<String> {
        if max_len == 0 {
            bail!("read_c_string requires max_len > 0");
        }
        let bytes = self.read_bytes(address, max_len)?;
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(bytes.len());
        Ok(String::from_utf8_lossy(&bytes[..end]).to_string())
    }
}

#[derive(Debug, Clone)]
pub struct DeadlockedStyleOffsets {
    pub entity_root: usize,
    pub local_player_controller: usize,
    pub controller_steam_id: u64,
    pub controller_name: u64,
    pub controller_sanitized_name: Option<u64>,
    pub controller_pawn: u64,
    pub controller_score: u64,
    pub controller_inventory_services: u64,
    pub inventory_persona_public_level: u64,
    pub pawn_health: u64,
    pub pawn_team: u64,
    pub pawn_life_state: u64,
    pub pawn_game_scene_node: u64,
    pub pawn_eye_offset: u64,
    pub pawn_eye_angles: u64,
    pub pawn_view_angles: u64,
    pub pawn_shots_fired: u64,
    pub pawn_aim_punch_angle: u64,
    pub pawn_aim_punch_services: u64,
    pub pawn_crosshair_entity: u64,
    pub pawn_velocity: u64,
    pub pawn_flash_duration: u64,
    pub pawn_is_scoped: u64,
    pub pawn_deathmatch_immunity: u64,
    pub game_scene_node_dormant: u64,
    pub game_scene_node_origin: u64,
    pub game_scene_node_model_state: u64,
    pub model_state_skeleton_instance: u64,
    pub aim_punch_cache: u64,
    pub entity_identity_size: usize,
}

#[derive(Debug, Clone)]
pub struct LocalPlayerRecord {
    pub controller: usize,
    pub pawn: usize,
    pub health: i32,
    pub team_num: i32,
    pub life_state: i32,
    pub shots_fired: i32,
    pub deathmatch_immunity: bool,
    pub origin: [f32; 3],
    pub eye_position: [f32; 3],
    pub view_angles: [f32; 2],
    pub aim_punch_angle: [f32; 2],
    pub velocity: [f32; 3],
    pub flash_duration: f32,
    pub is_scoped: bool,
    pub crosshair_entity_index: i32,
}

#[derive(Debug, Clone)]
pub struct RadarPlayerRecord {
    pub controller: usize,
    pub pawn: usize,
    pub name: String,
    pub steam_id: u64,
    pub health: i32,
    pub team_num: i32,
    pub life_state: i32,
    pub origin: [f32; 3],
    pub dormant: bool,
    pub deathmatch_immunity: bool,
    pub is_local: bool,
}

#[derive(Debug, Clone)]
pub struct SchemaSystem {
    scopes: BTreeMap<String, ModuleScope>,
}

#[derive(Debug, Clone)]
pub struct ModuleScope {
    name: String,
    classes: BTreeMap<String, Class>,
}

#[derive(Debug, Clone)]
pub struct Class {
    name: String,
    fields: BTreeMap<String, u64>,
    size: i32,
}

#[derive(Debug, Clone)]
struct Field {
    name: String,
    offset: u64,
}

impl SchemaSystem {
    pub fn read<R: HeadlessRuntime>(runtime: &mut R) -> Result<Self> {
        let schema_match = runtime
            .scan_module_memory_pattern("libschemasystem.so", SCHEMA_SYSTEM_PATTERN)?
            .context("failed to locate schema system pointer in libschemasystem.so")?;
        let schema_system = runtime
            .read_relative_address(schema_match, 3, 7)
            .context("decode SchemaSystem RIP")?;
        let type_scopes_len = runtime
            .read_i32(checked_addr(
                schema_system,
                0x1F0,
                "read SchemaSystem type_scopes_len",
            )?)
            .context("read SchemaSystem type_scopes_len")?;
        let type_scopes_vec = runtime
            .read_u64(checked_addr(
                schema_system,
                0x1F8,
                "read SchemaSystem type_scopes_vec",
            )?)
            .context("read SchemaSystem type_scopes_vec")? as usize;

        let mut scopes = BTreeMap::new();
        for idx in 0..type_scopes_len.max(0) as usize {
            let Ok(type_scope_address) = runtime.read_u64(checked_addr(
                type_scopes_vec,
                checked_mul(idx, 8, "SchemaSystem type scope pointer index")?,
                "read SchemaSystem type scope pointer",
            )?) else {
                continue;
            };
            let type_scope_address = type_scope_address as usize;
            if type_scope_address == 0 {
                continue;
            }
            let Ok(scope) = ModuleScope::read(runtime, type_scope_address) else {
                continue;
            };
            if scope.name.is_empty() {
                continue;
            }
            scopes.insert(scope.name.clone(), scope);
        }

        Ok(Self { scopes })
    }

    pub fn get_library(&self, library: &str) -> Option<&ModuleScope> {
        self.scopes.get(library)
    }
}

impl ModuleScope {
    fn read<R: HeadlessRuntime>(runtime: &mut R, address: usize) -> Result<Self> {
        let name = runtime
            .read_c_string(checked_addr(address, 0x08, "read ModuleScope name")?, 128)
            .unwrap_or_default();
        let mut classes = BTreeMap::new();
        let hash_vector = checked_addr(address, 0x560 + 0x90, "read ModuleScope hash vector")?;

        for bucket in 0..1024usize {
            let bucket_offset = checked_mul(bucket, 24, "ModuleScope bucket stride")?
                .checked_add(0x28)
                .context("ModuleScope bucket offset overflow")?;
            let Ok(current_element) = runtime.read_u64(checked_addr(
                hash_vector,
                bucket_offset,
                "read ModuleScope bucket element",
            )?) else {
                continue;
            };
            let mut current_element = current_element as usize;
            while current_element != 0 {
                let Ok(data) = runtime.read_u64(checked_addr(
                    current_element,
                    0x10,
                    "read ModuleScope class data pointer",
                )?) else {
                    break;
                };
                let data = data as usize;
                if data != 0 {
                    if let Ok(class) = Class::read(runtime, data) {
                        classes.insert(class.name.clone(), class);
                    }
                }
                let Ok(next) = runtime.read_u64(checked_addr(
                    current_element,
                    0x08,
                    "read ModuleScope next element pointer",
                )?) else {
                    break;
                };
                current_element = next as usize;
            }
        }

        let Ok(current_blob) = runtime.read_u64(checked_addr(
            address,
            0x560 + 0x20,
            "read ModuleScope blob list",
        )?) else {
            return Ok(Self { name, classes });
        };
        let mut current_blob = current_blob as usize;
        while current_blob != 0 {
            let Ok(data) = runtime.read_u64(checked_addr(
                current_blob,
                0x10,
                "read ModuleScope blob class pointer",
            )?) else {
                break;
            };
            let data = data as usize;
            if let Ok(class) = Class::read(runtime, data) {
                classes.insert(class.name.clone(), class);
            }
            let Ok(next) = runtime.read_u64(current_blob) else {
                break;
            };
            current_blob = next as usize;
        }

        Ok(Self { name, classes })
    }

    pub fn get_class(&self, class: &str) -> Option<&Class> {
        self.classes.get(class)
    }
}

impl Class {
    fn read<R: HeadlessRuntime>(runtime: &mut R, address: usize) -> Result<Self> {
        let name_ptr =
            runtime.read_u64(checked_addr(address, 0x08, "read Class name pointer")?)? as usize;
        let name = runtime.read_c_string(name_ptr, 128)?;
        let field_count =
            runtime.read_i16(checked_addr(address, 0x24, "read Class field count")?)?;
        let size = runtime.read_i32(checked_addr(address, 0x20, "read Class size")?)?;
        if !(0..=20_000).contains(&field_count) {
            return Ok(Self {
                name,
                fields: BTreeMap::new(),
                size,
            });
        }

        let fields_vec =
            runtime.read_u64(checked_addr(address, 0x30, "read Class fields vector")?)? as usize;
        let mut fields = BTreeMap::new();
        for idx in 0..field_count as usize {
            let field_address = checked_addr(
                fields_vec,
                checked_mul(idx, 0x20, "Class field stride")?,
                "read Class field entry",
            )?;
            if let Ok(field) = Field::read(runtime, field_address) {
                fields.insert(field.name, field.offset);
            }
        }

        Ok(Self { name, fields, size })
    }

    pub fn get(&self, field: &str) -> Option<u64> {
        self.fields.get(field).copied()
    }

    pub fn size(&self) -> i32 {
        self.size
    }
}

impl Field {
    fn read<R: HeadlessRuntime>(runtime: &mut R, address: usize) -> Result<Self> {
        let name_ptr = runtime.read_u64(address)? as usize;
        let name = runtime.read_c_string(name_ptr, 128)?;
        let offset = runtime.read_i32(checked_addr(address, 0x10, "read Field offset")?)? as u64;
        Ok(Self { name, offset })
    }
}

pub fn find_deadlocked_style_offsets<R: HeadlessRuntime>(
    runtime: &mut R,
) -> Result<DeadlockedStyleOffsets> {
    let _client = runtime.module_image("libclient.so")?;
    let local_player_controller = runtime
        .scan_module_memory_pattern("libclient.so", "48 83 3D ? ? ? ? 00 0F 95 C0 C3")?
        .context("missing local player controller pattern")?;
    let local_player_controller = runtime
        .read_relative_address(local_player_controller, 3, 8)
        .context("decode local player controller RIP")?;

    let resource =
        resolve_interface_instance(runtime, "libengine2.so", "GameResourceServiceClientV0")?
            .context("missing GameResourceServiceClientV0 interface")?;
    let entity_root = runtime
        .read_u64(checked_addr(
            resource,
            0x50,
            "read GameResourceServiceClient entity root pointer",
        )?)
        .context("read GameResourceServiceClient entity root pointer")?
        as usize;
    let entity_root = checked_addr(entity_root, 0x10, "compute entity root offset")?;

    let schema = SchemaSystem::read(runtime).context("read live SchemaSystem")?;
    let client_scope = schema
        .get_library("libclient.so")
        .context("missing libclient.so schema scope")?;
    let entity_identity_size = client_scope
        .get_class("CEntityIdentity")
        .context("missing CEntityIdentity schema class")?
        .size() as usize;

    let controller_steam_id = client_scope
        .get_class("CBasePlayerController")
        .and_then(|class| class.get("m_steamID"))
        .or_else(|| {
            client_scope
                .get_class("CCSPlayerController")
                .and_then(|class| class.get("m_steamID"))
        })
        .context("missing player controller steam id field")?;
    let controller_name = client_scope
        .get_class("CBasePlayerController")
        .and_then(|class| class.get("m_iszPlayerName"))
        .or_else(|| {
            client_scope
                .get_class("CCSPlayerController")
                .and_then(|class| class.get("m_iszPlayerName"))
        })
        .context("missing player controller name field")?;
    let controller_sanitized_name = client_scope
        .get_class("CCSPlayerController")
        .and_then(|class| class.get("m_sSanitizedPlayerName"));
    let controller_pawn = client_scope
        .get_class("CBasePlayerController")
        .and_then(|class| class.get("m_hPawn"))
        .or_else(|| {
            client_scope
                .get_class("CCSPlayerController")
                .and_then(|class| class.get("m_hPawn"))
        })
        .context("missing player controller pawn handle field")?;
    let controller_score = client_scope
        .get_class("CCSPlayerController")
        .and_then(|class| class.get("m_iScore"))
        .context("missing CCSPlayerController::m_iScore")?;
    let controller_inventory_services = client_scope
        .get_class("CCSPlayerController")
        .and_then(|class| class.get("m_pInventoryServices"))
        .context("missing CCSPlayerController::m_pInventoryServices")?;
    let inventory_persona_public_level = client_scope
        .get_class("CCSPlayerController_InventoryServices")
        .and_then(|class| class.get("m_nPersonaDataPublicLevel"))
        .context("missing CCSPlayerController_InventoryServices::m_nPersonaDataPublicLevel")?;
    let pawn_health = client_scope
        .get_class("C_BaseEntity")
        .and_then(|class| class.get("m_iHealth"))
        .context("missing C_BaseEntity::m_iHealth")?;
    let pawn_team = client_scope
        .get_class("C_BaseEntity")
        .and_then(|class| class.get("m_iTeamNum"))
        .context("missing C_BaseEntity::m_iTeamNum")?;
    let pawn_life_state = client_scope
        .get_class("C_BaseEntity")
        .and_then(|class| class.get("m_lifeState"))
        .context("missing C_BaseEntity::m_lifeState")?;
    let pawn_game_scene_node = client_scope
        .get_class("C_BaseEntity")
        .and_then(|class| class.get("m_pGameSceneNode"))
        .context("missing C_BaseEntity::m_pGameSceneNode")?;
    let pawn_eye_offset = client_scope
        .get_class("C_BaseModelEntity")
        .and_then(|class| class.get("m_vecViewOffset"))
        .context("missing C_BaseModelEntity::m_vecViewOffset")?;
    let pawn_eye_angles = client_scope
        .get_class("C_CSPlayerPawn")
        .and_then(|class| class.get("m_angEyeAngles"))
        .context("missing C_CSPlayerPawn::m_angEyeAngles")?;
    let pawn_view_angles = client_scope
        .get_class("C_BasePlayerPawn")
        .and_then(|class| class.get("v_angle"))
        .context("missing C_BasePlayerPawn::v_angle")?;
    let pawn_shots_fired = client_scope
        .get_class("C_CSPlayerPawn")
        .and_then(|class| class.get("m_iShotsFired"))
        .context("missing C_CSPlayerPawn::m_iShotsFired")?;
    let pawn_aim_punch_angle = client_scope
        .get_class("C_CSPlayerPawn")
        .and_then(|class| class.get("m_aimPunchAngle"))
        .unwrap_or_default();
    let pawn_aim_punch_services = client_scope
        .get_class("C_CSPlayerPawn")
        .and_then(|class| class.get("m_pAimPunchServices"))
        .context("missing C_CSPlayerPawn::m_pAimPunchServices")?;
    let pawn_crosshair_entity = client_scope
        .get_class("C_CSPlayerPawn")
        .and_then(|class| class.get("m_iIDEntIndex"))
        .context("missing C_CSPlayerPawn::m_iIDEntIndex")?;
    let pawn_velocity = client_scope
        .get_class("C_BaseEntity")
        .and_then(|class| class.get("m_vecAbsVelocity"))
        .context("missing C_BaseEntity::m_vecAbsVelocity")?;
    let pawn_flash_duration = client_scope
        .get_class("C_CSPlayerPawnBase")
        .and_then(|class| class.get("m_flFlashDuration"))
        .context("missing C_CSPlayerPawnBase::m_flFlashDuration")?;
    let pawn_is_scoped = client_scope
        .get_class("C_CSPlayerPawn")
        .and_then(|class| class.get("m_bIsScoped"))
        .context("missing C_CSPlayerPawn::m_bIsScoped")?;
    let pawn_deathmatch_immunity = client_scope
        .get_class("C_CSPlayerPawn")
        .and_then(|class| class.get("m_bGunGameImmunity"))
        .context("missing C_CSPlayerPawn::m_bGunGameImmunity")?;
    let game_scene_node_dormant = client_scope
        .get_class("CGameSceneNode")
        .and_then(|class| class.get("m_bDormant"))
        .context("missing CGameSceneNode::m_bDormant")?;
    let game_scene_node_origin = client_scope
        .get_class("CGameSceneNode")
        .and_then(|class| class.get("m_vecAbsOrigin"))
        .context("missing CGameSceneNode::m_vecAbsOrigin")?;
    let game_scene_node_model_state = client_scope
        .get_class("CSkeletonInstance")
        .and_then(|class| class.get("m_modelState"))
        .context("missing CSkeletonInstance::m_modelState")?;
    let model_state_skeleton_instance = client_scope
        .get_class("CBodyComponentSkeletonInstance")
        .and_then(|class| class.get("m_skeletonInstance"))
        .context("missing CBodyComponentSkeletonInstance::m_skeletonInstance")?;
    let aim_punch_cache = client_scope
        .get_class("CCSPlayer_AimPunchServices")
        .and_then(|class| class.get("m_unpredictableBaseTick"))
        .context("missing CCSPlayer_AimPunchServices::m_unpredictableBaseTick")?
        .saturating_sub(0x18);

    Ok(DeadlockedStyleOffsets {
        entity_root,
        local_player_controller,
        controller_steam_id,
        controller_name,
        controller_sanitized_name,
        controller_pawn,
        controller_score,
        controller_inventory_services,
        inventory_persona_public_level,
        pawn_health,
        pawn_team,
        pawn_life_state,
        pawn_game_scene_node,
        pawn_eye_offset,
        pawn_eye_angles,
        pawn_view_angles,
        pawn_shots_fired,
        pawn_aim_punch_angle,
        pawn_aim_punch_services,
        pawn_crosshair_entity,
        pawn_velocity,
        pawn_flash_duration,
        pawn_is_scoped,
        pawn_deathmatch_immunity,
        game_scene_node_dormant,
        game_scene_node_origin,
        game_scene_node_model_state,
        model_state_skeleton_instance,
        aim_punch_cache,
        entity_identity_size,
    })
}

pub fn read_local_player_record<R: HeadlessRuntime>(
    runtime: &mut R,
    offsets: &DeadlockedStyleOffsets,
) -> Result<Option<LocalPlayerRecord>> {
    let controller = runtime.read_u64(offsets.local_player_controller)? as usize;
    if controller == 0 {
        return Ok(None);
    }

    let pawn_handle = runtime.read_u32(checked_addr(
        controller,
        offsets.controller_pawn as usize,
        "read local pawn handle",
    )?)?;
    if pawn_handle == u32::MAX {
        return Ok(None);
    }
    let Some(pawn) = resolve_entity_handle(runtime, offsets, pawn_handle as i32)? else {
        return Ok(None);
    };

    let game_scene_node = runtime.read_u64(checked_addr(
        pawn,
        offsets.pawn_game_scene_node as usize,
        "read pawn game scene node",
    )?)? as usize;
    if game_scene_node == 0 {
        return Ok(None);
    }

    let origin = read_vec3(
        runtime,
        checked_addr(
            game_scene_node,
            offsets.game_scene_node_origin as usize,
            "read game scene node origin",
        )?,
    )?;
    let eye_offset = read_vec3(
        runtime,
        checked_addr(
            pawn,
            offsets.pawn_eye_offset as usize,
            "read pawn eye offset",
        )?,
    )?;
    let view_angles = [
        runtime.read_f32(checked_addr(
            pawn,
            offsets.pawn_view_angles as usize,
            "read pawn view angles",
        )?)?,
        runtime.read_f32(checked_addr(
            checked_addr(
                pawn,
                offsets.pawn_view_angles as usize,
                "read pawn view angles base",
            )?,
            4,
            "read pawn view angles second component",
        )?)?,
    ];
    let velocity = read_vec3(
        runtime,
        checked_addr(pawn, offsets.pawn_velocity as usize, "read pawn velocity")?,
    )?;
    let flash_duration = runtime.read_f32(checked_addr(
        pawn,
        offsets.pawn_flash_duration as usize,
        "read pawn flash duration",
    )?)?;
    let is_scoped = runtime.read_u8(checked_addr(
        pawn,
        offsets.pawn_is_scoped as usize,
        "read pawn scoped",
    )?)? != 0;
    let shots_fired = runtime.read_i32(checked_addr(
        pawn,
        offsets.pawn_shots_fired as usize,
        "read pawn shots fired",
    )?)?;
    let aim_punch_angle = read_aim_punch_vec2(runtime, pawn, offsets).unwrap_or([0.0, 0.0]);
    let crosshair_entity_index = runtime.read_i32(checked_addr(
        pawn,
        offsets.pawn_crosshair_entity as usize,
        "read pawn crosshair entity",
    )?)?;

    Ok(Some(LocalPlayerRecord {
        controller,
        pawn,
        health: runtime.read_i32(checked_addr(
            pawn,
            offsets.pawn_health as usize,
            "read pawn health",
        )?)?,
        team_num: runtime.read_i32(checked_addr(
            pawn,
            offsets.pawn_team as usize,
            "read pawn team",
        )?)?,
        life_state: i32::from(runtime.read_u8(checked_addr(
            pawn,
            offsets.pawn_life_state as usize,
            "read pawn life state",
        )?)?),
        shots_fired,
        deathmatch_immunity: runtime.read_u8(checked_addr(
            pawn,
            offsets.pawn_deathmatch_immunity as usize,
            "read pawn deathmatch immunity",
        )?)? != 0,
        origin,
        eye_position: [
            origin[0] + eye_offset[0],
            origin[1] + eye_offset[1],
            origin[2] + eye_offset[2],
        ],
        view_angles,
        aim_punch_angle,
        velocity,
        flash_duration,
        is_scoped,
        crosshair_entity_index,
    }))
}

pub fn read_radar_player_records<R: HeadlessRuntime>(
    runtime: &mut R,
    offsets: &DeadlockedStyleOffsets,
) -> Result<Vec<RadarPlayerRecord>> {
    const NUM_BUCKETS: usize = 64;
    const IDENTITIES_PER_BUCKET: usize = 512;
    const PLAYER_CONTROLLER_CLASS: &str = "19CCSPlayerController";

    let local_controller = runtime.read_u64(offsets.local_player_controller)? as usize;
    let local_pawn_handle = if local_controller == 0 {
        u32::MAX
    } else {
        runtime.read_u32(checked_addr(
            local_controller,
            offsets.controller_pawn as usize,
            "read local controller pawn handle",
        )?)?
    };
    let local_pawn = if local_pawn_handle == u32::MAX {
        None
    } else {
        resolve_entity_handle(runtime, offsets, local_pawn_handle as i32)?
    };

    let mut players = Vec::new();
    for bucket_index in 0..NUM_BUCKETS {
        let bucket_ptr = runtime.read_u64(checked_addr(
            offsets.entity_root,
            checked_mul(bucket_index, 8, "entity root bucket index")?,
            "read entity root bucket pointer",
        )?)? as usize;
        if bucket_ptr == 0 || (bucket_ptr >> 48) != 0 {
            continue;
        }

        let bucket = runtime.read_bytes(
            bucket_ptr,
            IDENTITIES_PER_BUCKET
                .checked_mul(offsets.entity_identity_size)
                .context("entity identity bucket size overflow")?,
        )?;

        for index_in_bucket in 0..IDENTITIES_PER_BUCKET {
            let identity_offset = index_in_bucket * offsets.entity_identity_size;
            let entity = read_le_u64(&bucket, identity_offset)?;
            if entity == 0 {
                continue;
            }

            let handle = read_le_u32(
                &bucket,
                identity_offset
                    .checked_add(0x10)
                    .context("identity handle offset overflow")?,
            )?;
            let handle_index = handle & 0x7FFF;
            let entity_index = bucket_index
                .checked_mul(IDENTITIES_PER_BUCKET)
                .and_then(|value| value.checked_add(index_in_bucket))
                .context("entity index overflow")? as u32;
            if entity_index != handle_index {
                continue;
            }

            let Some(class_name) = read_instance_class_name(runtime, entity as usize)? else {
                continue;
            };
            if class_name != PLAYER_CONTROLLER_CLASS {
                continue;
            }

            let controller = entity as usize;
            let pawn_handle = runtime.read_u32(checked_addr(
                controller,
                offsets.controller_pawn as usize,
                "read controller pawn handle",
            )?)?;
            if pawn_handle == u32::MAX {
                continue;
            }
            let Some(pawn) = resolve_entity_handle(runtime, offsets, pawn_handle as i32)? else {
                continue;
            };

            let game_scene_node = runtime.read_u64(checked_addr(
                pawn,
                offsets.pawn_game_scene_node as usize,
                "read pawn game scene node",
            )?)? as usize;
            if game_scene_node == 0 {
                continue;
            }

            let health = runtime.read_i32(checked_addr(
                pawn,
                offsets.pawn_health as usize,
                "read pawn health",
            )?)?;
            let team_num = runtime.read_i32(checked_addr(
                pawn,
                offsets.pawn_team as usize,
                "read pawn team",
            )?)?;
            let life_state = i32::from(runtime.read_u8(checked_addr(
                pawn,
                offsets.pawn_life_state as usize,
                "read pawn life state",
            )?)?);
            let deathmatch_immunity = runtime.read_u8(checked_addr(
                pawn,
                offsets.pawn_deathmatch_immunity as usize,
                "read pawn deathmatch immunity",
            )?)? != 0;
            let dormant = runtime.read_u8(checked_addr(
                game_scene_node,
                offsets.game_scene_node_dormant as usize,
                "read game scene node dormant",
            )?)? != 0;
            let origin = read_vec3(
                runtime,
                checked_addr(
                    game_scene_node,
                    offsets.game_scene_node_origin as usize,
                    "read game scene node origin",
                )?,
            )?;
            let steam_id = runtime.read_u64(checked_addr(
                controller,
                offsets.controller_steam_id as usize,
                "read controller steam id",
            )?)?;
            let name = read_controller_name(runtime, controller, offsets)?;

            players.push(RadarPlayerRecord {
                controller,
                pawn,
                name,
                steam_id,
                health,
                team_num,
                life_state,
                origin,
                dormant,
                deathmatch_immunity,
                is_local: local_pawn == Some(pawn),
            });
        }
    }

    Ok(players)
}

fn resolve_entity_handle<R: HeadlessRuntime>(
    runtime: &mut R,
    offsets: &DeadlockedStyleOffsets,
    handle: i32,
) -> Result<Option<usize>> {
    let index = handle as u32 & 0x7FFF;
    resolve_entity_index(runtime, offsets, index)
}

fn resolve_entity_index<R: HeadlessRuntime>(
    runtime: &mut R,
    offsets: &DeadlockedStyleOffsets,
    index: u32,
) -> Result<Option<usize>> {
    let bucket_index = (index >> 9) as usize;
    let index_in_bucket = (index & 0x1FF) as usize;
    let bucket_ptr = runtime.read_u64(checked_addr(
        offsets.entity_root,
        checked_mul(bucket_index, 8, "entity root bucket index")?,
        "read entity root bucket pointer",
    )?)? as usize;
    if bucket_ptr == 0 {
        return Ok(None);
    }
    let entity = runtime.read_u64(checked_addr(
        bucket_ptr,
        checked_mul(
            index_in_bucket,
            offsets.entity_identity_size,
            "entity identity stride",
        )?,
        "read entity by index",
    )?)? as usize;
    if entity == 0 {
        return Ok(None);
    }
    Ok(Some(entity))
}

fn read_controller_name<R: HeadlessRuntime>(
    runtime: &mut R,
    controller: usize,
    offsets: &DeadlockedStyleOffsets,
) -> Result<String> {
    if let Some(name) = read_sanitized_controller_name(runtime, controller, offsets) {
        return Ok(name);
    }

    let Ok(name_ptr) = runtime.read_u64(checked_addr(
        controller,
        offsets.controller_name as usize,
        "read controller name pointer",
    )?) else {
        return Ok(String::new());
    };
    let name_ptr = name_ptr as usize;
    if name_ptr == 0 {
        return Ok(String::new());
    }

    let Ok(name) = runtime.read_c_string(name_ptr, 128) else {
        return Ok(String::new());
    };
    Ok(name.trim().to_string())
}

fn read_sanitized_controller_name<R: HeadlessRuntime>(
    runtime: &mut R,
    controller: usize,
    offsets: &DeadlockedStyleOffsets,
) -> Option<String> {
    let sanitized_offset = offsets.controller_sanitized_name?;
    let name_addr = checked_addr(
        controller,
        sanitized_offset as usize,
        "read sanitized controller name pointer",
    )
    .ok()?;
    let name_ptr = runtime.read_u64(name_addr).ok()? as usize;
    if name_ptr == 0 {
        return None;
    }
    let name = runtime.read_c_string(name_ptr, 128).ok()?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

fn read_instance_class_name<R: HeadlessRuntime>(
    runtime: &mut R,
    instance_runtime: usize,
) -> Result<Option<String>> {
    let vtable = runtime.read_u64(instance_runtime)? as usize;
    if vtable < 8 {
        return Ok(None);
    }
    let typeinfo = runtime.read_u64(vtable.checked_sub(8).context("vtable underflow")?)? as usize;
    if typeinfo == 0 {
        return Ok(None);
    }
    let name_ptr =
        runtime.read_u64(checked_addr(typeinfo, 8, "read typeinfo name pointer")?)? as usize;
    if name_ptr == 0 {
        return Ok(None);
    }
    Ok(Some(runtime.read_c_string(name_ptr, 128)?))
}

fn read_vec3<R: HeadlessRuntime>(runtime: &mut R, address: usize) -> Result<[f32; 3]> {
    Ok([
        runtime.read_f32(address)?,
        runtime.read_f32(checked_addr(address, 4, "read vec3 y")?)?,
        runtime.read_f32(checked_addr(address, 8, "read vec3 z")?)?,
    ])
}

fn read_aim_punch_vec2<R: HeadlessRuntime>(
    runtime: &mut R,
    pawn: usize,
    offsets: &DeadlockedStyleOffsets,
) -> Result<[f32; 2]> {
    let aim_punch_services = runtime.read_u64(checked_addr(
        pawn,
        offsets.pawn_aim_punch_services as usize,
        "read pawn aim punch services",
    )?)? as usize;
    if aim_punch_services == 0 {
        return Ok([0.0, 0.0]);
    }

    let length = runtime.read_u64(checked_addr(
        aim_punch_services,
        offsets.aim_punch_cache as usize,
        "read aim punch cache length",
    )?)? as usize;
    if length == 0 {
        return Ok([0.0, 0.0]);
    }

    let data_ptr_addr = checked_addr(
        aim_punch_services,
        offsets
            .aim_punch_cache
            .checked_add(0x08)
            .context("aim punch cache data pointer overflow")? as usize,
        "read aim punch cache data pointer",
    )?;
    let data_ptr = runtime.read_u64(data_ptr_addr)? as usize;
    if data_ptr == 0 {
        return Ok([0.0, 0.0]);
    }

    let sample_index = length.saturating_sub(1);
    let sample_offset = checked_mul(sample_index, 12, "aim punch sample stride")?;
    let sample_addr = checked_addr(data_ptr, sample_offset, "read aim punch sample")?;
    Ok([
        runtime.read_f32(sample_addr)?,
        runtime.read_f32(checked_addr(sample_addr, 4, "read aim punch sample yaw")?)?,
    ])
}

fn read_le_u64(bytes: &[u8], offset: usize) -> Result<u64> {
    let end = offset
        .checked_add(8)
        .context("invalid u64 slice offset overflow")?;
    Ok(u64::from_le_bytes(
        bytes[offset..end].try_into().context("invalid u64 slice")?,
    ))
}

fn read_le_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let end = offset
        .checked_add(4)
        .context("invalid u32 slice offset overflow")?;
    Ok(u32::from_le_bytes(
        bytes[offset..end].try_into().context("invalid u32 slice")?,
    ))
}

fn resolve_interface_instance<R: HeadlessRuntime>(
    runtime: &mut R,
    module_suffix: &str,
    interface_name: &str,
) -> Result<Option<usize>> {
    let module = runtime.module_image(module_suffix)?;
    let file_bytes = runtime.read_module_file(module_suffix)?;
    let entries = read_interfaces(runtime, module_suffix, &module, &file_bytes)?;
    let Some(entry) = entries
        .into_iter()
        .find(|entry| entry.name.starts_with(interface_name))
    else {
        return Ok(None);
    };
    decode_interface_instance_runtime(&module, &file_bytes, &entry)
}

fn read_interfaces<R: HeadlessRuntime>(
    runtime: &mut R,
    module_suffix: &str,
    module: &ModuleImage,
    file_bytes: &[u8],
) -> Result<Vec<InterfaceEntry>> {
    let create_interface_virtual =
        find_symbol_virtual_addr(file_bytes, CREATE_INTERFACE_SYMBOL)?.map(|value| value as usize);
    let interface_list_virtual = find_interface_list_virtual(file_bytes, create_interface_virtual)?
        .with_context(|| {
            format!("failed to discover InterfaceReg list in module `{module_suffix}`")
        })?;

    let interface_list_runtime = checked_addr(
        module.image_base,
        interface_list_virtual,
        "compute interface list runtime address",
    )?;
    let list_head = runtime.read_u64(interface_list_runtime)? as usize;
    read_interface_entries(runtime, list_head, 256)
}

fn decode_interface_instance_runtime(
    module: &ModuleImage,
    file_bytes: &[u8],
    entry: &InterfaceEntry,
) -> Result<Option<usize>> {
    let create_fn_virtual = entry
        .create_fn
        .checked_sub(module.image_base)
        .context("create_fn is below module image base")?;
    let Some(file_offset) = file_offset_for_virtual_addr(file_bytes, create_fn_virtual as u64)?
    else {
        return Ok(None);
    };
    let file_offset = file_offset as usize;
    let file_end = file_offset
        .checked_add(8)
        .context("create_fn offset overflow for interface instance decode")?;
    let code = file_bytes
        .get(file_offset..file_end)
        .context("create_fn offset is outside file bounds for interface instance decode")?;
    if !code.starts_with(CREATE_FN_RET_PATTERN) || code.get(7).copied() != Some(0xC3) {
        return Ok(None);
    }

    let disp = i32::from_le_bytes([code[3], code[4], code[5], code[6]]) as i64;
    let create_fn_after_ret = create_fn_virtual
        .checked_add(7)
        .context("create_fn virtual address overflow")?;
    let instance_virtual = ((create_fn_after_ret as i64) + disp) as usize;
    Ok(Some(checked_addr(
        module.image_base,
        instance_virtual,
        "compute interface instance runtime address",
    )?))
}

fn find_interface_list_virtual(
    file_bytes: &[u8],
    create_interface_virtual: Option<usize>,
) -> Result<Option<usize>> {
    let pattern = parse_pattern(INTERFACE_LIST_PATTERN)?;

    if let Some(symbol_virtual) = create_interface_virtual {
        if let Some(file_offset) = file_offset_for_virtual_addr(file_bytes, symbol_virtual as u64)?
        {
            let window_start = file_offset as usize;
            let window_end = window_start
                .checked_add(0x40)
                .context("interface list window overflow")?
                .min(file_bytes.len());
            let local_matches = find_matches(&file_bytes[window_start..window_end], &pattern, 1);
            if let Some(local) = local_matches.first() {
                let match_virtual = symbol_virtual
                    .checked_add(*local)
                    .context("interface list local match overflow")?;
                let local_start = window_start
                    .checked_add(*local)
                    .context("interface list local slice overflow")?;
                return Ok(Some(decode_rip_target(
                    match_virtual,
                    &file_bytes[local_start..],
                )?));
            }
        }
    }

    let matches = find_matches(file_bytes, &pattern, 4);
    let Some(file_match) = matches.first().copied() else {
        return Ok(None);
    };
    Ok(Some(decode_rip_target(
        file_match,
        &file_bytes[file_match..],
    )?))
}

fn decode_rip_target(virtual_match_offset: usize, bytes: &[u8]) -> Result<usize> {
    let disp = i32::from_le_bytes(
        bytes
            .get(3..7)
            .context("pattern match too short to decode RIP displacement")?
            .try_into()
            .context("invalid RIP displacement slice")?,
    ) as i64;
    let after_ret = virtual_match_offset
        .checked_add(7)
        .context("RIP target virtual offset overflow")?;
    Ok(((after_ret as i64) + disp) as usize)
}

fn read_interface_entries<R: HeadlessRuntime>(
    runtime: &mut R,
    mut cursor: usize,
    max_entries: usize,
) -> Result<Vec<InterfaceEntry>> {
    let mut entries = Vec::new();

    while cursor != 0 && entries.len() < max_entries {
        let create_fn = runtime.read_u64(cursor)? as usize;
        let name_ptr =
            runtime.read_u64(checked_addr(cursor, 0x8, "read interface name pointer")?)? as usize;
        let next =
            runtime.read_u64(checked_addr(cursor, 0x10, "read interface next pointer")?)? as usize;
        let name = runtime.read_c_string(name_ptr, 128)?;
        if name.is_empty() {
            break;
        }

        entries.push(InterfaceEntry { name, create_fn });
        cursor = next;
    }

    Ok(entries)
}
