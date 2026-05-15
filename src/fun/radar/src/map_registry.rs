use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, bail};
use serde::Deserialize;

const OVERVIEW_TEXTURE_SIZE: f32 = 1024.0;

#[derive(Clone, Debug, Deserialize)]
pub struct MapTransform {
    pub pos_x: f32,
    pub pos_y: f32,
    pub scale: f32,
}

pub fn load_map_images(dir: &Path) -> anyhow::Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("png") {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some(map_key) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        if map_key == "default_png" {
            continue;
        }
        out.insert(map_key.to_string(), file_name.to_string());
    }
    if out.is_empty() {
        bail!("no radar png files found in {}", dir.display());
    }
    Ok(out)
}

pub fn load_map_transforms(dir: &Path) -> anyhow::Result<BTreeMap<String, MapTransform>> {
    let mut out = BTreeMap::new();
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Some(map_key) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        if map_key == "manifest" {
            continue;
        }
        let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let transform: MapTransform =
            serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
        out.insert(map_key.to_string(), transform);
    }
    if out.is_empty() {
        bail!("no radar transform json files found in {}", dir.display());
    }
    Ok(out)
}

pub fn normalize_map_key(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "maps/<empty>.vpk" {
        return None;
    }

    let without_prefix = trimmed.strip_prefix("maps/").unwrap_or(trimmed);
    let without_ext = without_prefix
        .strip_suffix(".vpk")
        .unwrap_or(without_prefix);
    if without_ext.is_empty() {
        None
    } else {
        Some(without_ext.to_string())
    }
}

pub fn world_to_radar(transform: &MapTransform, origin: [f32; 3]) -> [f32; 2] {
    let x = (origin[0] - transform.pos_x) / transform.scale / OVERVIEW_TEXTURE_SIZE;
    let y = (transform.pos_y - origin[1]) / transform.scale / OVERVIEW_TEXTURE_SIZE;
    [x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)]
}
