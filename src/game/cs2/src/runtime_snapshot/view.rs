use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};

use super::SnapshotOffsets;
use super::entities::{read_entity_abs_origin, vec3_plausible};
use super::memory::{SnapshotMemory, read_vec3, read_vec4};
use memreader_client::MemReaderModuleInfo;

const CVIEWRENDER_CLASS_NAME: &str = "CViewRender";
const CVIEWRENDER_CAMERA_ORIGIN_OFFSET: usize = 0x10;
const CVIEWRENDER_VIEW_ROW0_OFFSET: usize = 0x1E8;
const CVIEWRENDER_VIEW_ROW1_OFFSET: usize = 0x1F8;
const CVIEWRENDER_VIEW_ROW2_OFFSET: usize = 0x208;
const CVIEWRENDER_PROJ_SCALE_X_OFFSET: usize = 0x218;
const CVIEWRENDER_PROJ_SCALE_Y_OFFSET: usize = 0x22C;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ViewRenderCacheKey {
    pid: u32,
    module_end: usize,
}

#[derive(Debug, Clone, Copy)]
struct ViewRenderCacheEntry {
    vtable_addr: usize,
    instance_addr: usize,
}

static VIEW_RENDER_CACHE: OnceLock<Mutex<HashMap<ViewRenderCacheKey, ViewRenderCacheEntry>>> =
    OnceLock::new();

pub(super) fn read_view_matrix<B: SnapshotMemory>(
    runtime: &mut B,
    offsets: &SnapshotOffsets,
    local_pawn_addr: usize,
) -> Result<[f32; 16]> {
    let local_origin = read_entity_abs_origin(runtime, offsets, local_pawn_addr)?;
    let local_view_offset = read_vec3(runtime, local_pawn_addr + offsets.pawn_view_offset)?;
    let local_view_origin = [
        local_origin[0] + local_view_offset[0],
        local_origin[1] + local_view_offset[1],
        local_origin[2] + local_view_offset[2],
    ];
    let view_angles = [
        runtime.read_f32(local_pawn_addr + offsets.pawn_view_angles)?,
        runtime.read_f32(local_pawn_addr + offsets.pawn_view_angles + 4)?,
        0.0,
    ];

    let modules = runtime.mapped_modules();
    let client_module = modules
        .iter()
        .filter(|module| module.path.ends_with("libclient.so"))
        .max_by_key(|module| module.end)
        .context("missing libclient.so module")?;
    let view_render_addr =
        resolve_class_instance_addr(runtime, client_module.end, CVIEWRENDER_CLASS_NAME)?
            .context("failed to resolve CViewRender address")?;

    let camera_origin = [
        runtime.read_f32(view_render_addr + CVIEWRENDER_CAMERA_ORIGIN_OFFSET)?,
        runtime.read_f32(view_render_addr + CVIEWRENDER_CAMERA_ORIGIN_OFFSET + 4)?,
        runtime.read_f32(view_render_addr + CVIEWRENDER_CAMERA_ORIGIN_OFFSET + 8)?,
    ];
    anyhow::ensure!(
        vec3_plausible(camera_origin),
        "camera origin is not plausible"
    );

    let row0 = read_vec4(runtime, view_render_addr + CVIEWRENDER_VIEW_ROW0_OFFSET)?;
    let row1 = read_vec4(runtime, view_render_addr + CVIEWRENDER_VIEW_ROW1_OFFSET)?;
    let row2 = read_vec4(runtime, view_render_addr + CVIEWRENDER_VIEW_ROW2_OFFSET)?;
    let proj_scale_x = runtime.read_f32(view_render_addr + CVIEWRENDER_PROJ_SCALE_X_OFFSET)?;
    let proj_scale_y = runtime.read_f32(view_render_addr + CVIEWRENDER_PROJ_SCALE_Y_OFFSET)?;
    anyhow::ensure!(
        proj_scale_x.is_finite()
            && proj_scale_y.is_finite()
            && (0.1..=8.0).contains(&proj_scale_x.abs())
            && (0.1..=8.0).contains(&proj_scale_y.abs()),
        "projection scale is not plausible"
    );

    let matrix = [
        proj_scale_x * row0[0],
        proj_scale_x * row0[1],
        proj_scale_x * row0[2],
        proj_scale_x * row0[3],
        proj_scale_y * row1[0],
        proj_scale_y * row1[1],
        proj_scale_y * row1[2],
        proj_scale_y * row1[3],
        -row2[0],
        -row2[1],
        -row2[2],
        -row2[3],
        -row2[0],
        -row2[1],
        -row2[2],
        -row2[3],
    ];

    anyhow::ensure!(matrix_plausible(&matrix), "matrix is not plausible");
    anyhow::ensure!(
        forward_probe_projects_near_center(&matrix, camera_origin, view_angles, 128.0),
        "matrix forward probe failed"
    );
    let camera_origin_delta = [
        camera_origin[0] - local_view_origin[0],
        camera_origin[1] - local_view_origin[1],
        camera_origin[2] - local_view_origin[2],
    ];
    anyhow::ensure!(
        camera_origin_delta
            .into_iter()
            .all(|value| value.is_finite() && value.abs() < 256.0),
        "camera origin drift is too large"
    );
    Ok(matrix)
}

fn resolve_class_instance_addr<B: SnapshotMemory>(
    runtime: &mut B,
    module_end: usize,
    class_name: &str,
) -> Result<Option<usize>> {
    let cache = VIEW_RENDER_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let cache_key = ViewRenderCacheKey {
        pid: runtime.pid(),
        module_end,
    };
    if class_name == CVIEWRENDER_CLASS_NAME
        && let Ok(guard) = cache.lock()
        && let Some(entry) = guard.get(&cache_key).copied()
    {
        let cached_vtable = runtime
            .read_u64(entry.instance_addr)
            .ok()
            .map(|value| value as usize);
        let class_ok = cached_vtable == Some(entry.vtable_addr)
            && runtime
                .read_instance_class_name(entry.instance_addr)
                .ok()
                .flatten()
                .is_some_and(|name| name.contains(class_name));
        if class_ok {
            return Ok(Some(entry.instance_addr));
        }
    }

    let modules = runtime.mapped_modules();
    let module_entries = modules
        .iter()
        .filter(|entry| entry.path.ends_with("libclient.so"))
        .cloned()
        .collect::<Vec<_>>();
    let file_bytes = runtime
        .read_module_file("libclient.so")
        .context("failed to read module file libclient.so")?;

    let rtti_name = format!("{}{}", class_name.len(), class_name);
    let Some(name_file_offset) = file_bytes
        .windows(rtti_name.len())
        .position(|window| window == rtti_name.as_bytes())
    else {
        return Ok(None);
    };
    let name_addr = module_entries
        .iter()
        .find_map(|entry| {
            let region_size = entry.end.saturating_sub(entry.base);
            if name_file_offset >= entry.file_offset
                && name_file_offset < entry.file_offset + region_size
            {
                Some(entry.base + (name_file_offset - entry.file_offset))
            } else {
                None
            }
        })
        .context("failed to map RTTI file offset into runtime address")?;

    let Some(typeinfo_slot) =
        find_qword_in_mappings(runtime, &module_entries, name_addr as u64, 1, |_, _| true)?
            .into_iter()
            .next()
    else {
        return Ok(None);
    };
    if typeinfo_slot < 8 {
        return Ok(None);
    }
    let typeinfo_addr = typeinfo_slot - 8;
    let Some(vtable_slot) =
        find_qword_in_mappings(runtime, &module_entries, typeinfo_addr as u64, 1, |_, _| {
            true
        })?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };
    let vtable_addr = vtable_slot + 8;
    let mut instances = Vec::new();
    for (start, end) in find_nearby_anon_regions_from_modules(&modules, module_end, 0x800000) {
        instances.extend(find_qword_in_range(
            runtime,
            start,
            end,
            vtable_addr as u64,
            1,
        )?);
        if !instances.is_empty() {
            break;
        }
    }
    if instances.is_empty() {
        instances =
            find_qword_in_mappings(runtime, &modules, vtable_addr as u64, 1, |perms, path| {
                perms.starts_with("rw") && (path == "[anon]" || path == "[heap]")
            })?;
    }
    for instance_addr in instances {
        if runtime
            .read_instance_class_name(instance_addr)
            .ok()
            .flatten()
            .is_some_and(|name| name.contains(class_name))
        {
            if class_name == CVIEWRENDER_CLASS_NAME
                && let Ok(mut guard) = cache.lock()
            {
                guard.insert(
                    cache_key,
                    ViewRenderCacheEntry {
                        vtable_addr,
                        instance_addr,
                    },
                );
            }
            return Ok(Some(instance_addr));
        }
    }
    Ok(None)
}

fn find_nearby_anon_regions_from_modules(
    modules: &[MemReaderModuleInfo],
    module_end: usize,
    max_gap: usize,
) -> Vec<(usize, usize)> {
    let mut regions = Vec::new();
    for module in modules {
        if !module.perms.starts_with("rw") || module.path != "[anon]" {
            continue;
        }
        if module.base >= module_end && module.base.saturating_sub(module_end) <= max_gap {
            regions.push((module.base, module.end));
        }
    }
    regions
}

fn find_qword_in_range<B: SnapshotMemory>(
    runtime: &mut B,
    start: usize,
    end: usize,
    needle: u64,
    limit: usize,
) -> Result<Vec<usize>> {
    const CHUNK_SIZE: usize = 0x10000;
    if start >= end || limit == 0 {
        return Ok(Vec::new());
    }

    let needle_bytes = needle.to_le_bytes();
    let mut hits = Vec::new();
    let mut cursor = start;
    let mut carry = Vec::new();

    while cursor < end && hits.len() < limit {
        let chunk_end = (cursor + CHUNK_SIZE).min(end);
        let mut buf = vec![0; chunk_end - cursor];
        if runtime.read_exact_at(cursor, &mut buf).is_err() {
            cursor = chunk_end;
            carry.clear();
            continue;
        }

        let mut window = Vec::with_capacity(carry.len() + buf.len());
        window.extend_from_slice(&carry);
        window.extend_from_slice(&buf);
        for (index, bytes) in window.windows(8).enumerate() {
            if bytes == needle_bytes {
                let absolute = cursor.saturating_sub(carry.len()) + index;
                hits.push(absolute);
                if hits.len() >= limit {
                    break;
                }
            }
        }

        carry = window[window.len().saturating_sub(7)..].to_vec();
        cursor = chunk_end;
    }

    Ok(hits)
}

fn find_qword_in_mappings<B: SnapshotMemory>(
    runtime: &mut B,
    mappings: &[MemReaderModuleInfo],
    needle: u64,
    limit: usize,
    mut filter: impl FnMut(&str, &str) -> bool,
) -> Result<Vec<usize>> {
    const MAX_REGION_SIZE: usize = 0x1000000;
    let mut hits = Vec::new();
    for mapping in mappings {
        if !filter(&mapping.perms, &mapping.path) {
            continue;
        }
        if mapping.end <= mapping.base || mapping.end - mapping.base > MAX_REGION_SIZE {
            continue;
        }
        hits.extend(find_qword_in_range(
            runtime,
            mapping.base,
            mapping.end,
            needle,
            limit - hits.len(),
        )?);
        if hits.len() >= limit {
            break;
        }
    }
    Ok(hits)
}

fn matrix_plausible(matrix: &[f32; 16]) -> bool {
    let finite = matrix.iter().filter(|value| value.is_finite()).count();
    let nonzero = matrix
        .iter()
        .filter(|value| value.is_finite() && value.abs() > f32::EPSILON)
        .count();
    finite == 16 && nonzero >= 6 && matrix.iter().all(|value| value.abs() < 100_000.0)
}

fn clip_x(matrix: &[f32; 16], pos: [f32; 3]) -> f32 {
    matrix[0] * pos[0] + matrix[1] * pos[1] + matrix[2] * pos[2] + matrix[3]
}

fn clip_y(matrix: &[f32; 16], pos: [f32; 3]) -> f32 {
    matrix[4] * pos[0] + matrix[5] * pos[1] + matrix[6] * pos[2] + matrix[7]
}

fn clip_w(matrix: &[f32; 16], pos: [f32; 3]) -> f32 {
    matrix[12] * pos[0] + matrix[13] * pos[1] + matrix[14] * pos[2] + matrix[15]
}

fn forward_probe_projects_near_center(
    matrix: &[f32; 16],
    camera_origin: [f32; 3],
    eye_angles: [f32; 3],
    distance: f32,
) -> bool {
    let forward = forward_from_eye_angles(eye_angles);
    let probe = [
        camera_origin[0] + forward[0] * distance,
        camera_origin[1] + forward[1] * distance,
        camera_origin[2] + forward[2] * distance,
    ];
    let w = clip_w(matrix, probe);
    if !w.is_finite() || w <= 0.001 {
        return false;
    }
    let ndc_x = clip_x(matrix, probe) / w;
    let ndc_y = clip_y(matrix, probe) / w;
    ndc_x.abs() <= 1.25 && ndc_y.abs() <= 1.25
}

fn forward_from_eye_angles(eye_angles: [f32; 3]) -> [f32; 3] {
    let pitch = eye_angles[0].to_radians();
    let yaw = eye_angles[1].to_radians();
    let sp = pitch.sin();
    let cp = pitch.cos();
    let sy = yaw.sin();
    let cy = yaw.cos();
    [cp * cy, cp * sy, -sp]
}

pub(super) fn world_to_screen(matrix: &[f32; 16], pos: [f32; 3]) -> Option<[f32; 2]> {
    let x = clip_x(matrix, pos);
    let y = clip_y(matrix, pos);
    let w = clip_w(matrix, pos);
    if !w.is_finite() || w <= 0.001 {
        return None;
    }
    Some([x / w, y / w])
}
