use anyhow::{Context, Result};
use deadlocked_headless::ModuleImage;
use memreader_client::MemReaderModuleInfo;

use crate::elf::{file_offset_for_virtual_addr, find_symbol_virtual_addr};
use crate::pattern::{find_matches, parse_pattern};
use crate::runtime_snapshot::MemoryReader;

use super::host_runtime::MemReaderMemory;

const CREATE_INTERFACE_SYMBOL: &str = "CreateInterface";
const INTERFACE_LIST_PATTERN: &str = "48 8B 1D ? ? ? ? 48 85 DB 74 ?";
const CREATE_FN_RET_PATTERN: &[u8] = &[0x48, 0x8D, 0x05];

#[derive(Debug, Clone, PartialEq, Eq)]
struct InterfaceEntry {
    name: String,
    create_fn: usize,
}

pub(super) fn module_image(modules: &[MemReaderModuleInfo], suffix: &str) -> Option<ModuleImage> {
    let mut matches = modules
        .iter()
        .filter(|module| module.path.ends_with(suffix))
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return None;
    }

    matches.sort_unstable_by_key(|module| module.base);
    let path = matches.first()?.path.clone();
    let image_base = matches
        .iter()
        .filter(|module| module.file_offset == 0)
        .map(|module| module.base)
        .min()
        .unwrap_or(matches[0].base);
    let image_end = matches
        .iter()
        .map(|module| module.end)
        .max()
        .unwrap_or(matches[0].end);

    Some(ModuleImage {
        path,
        image_base,
        image_end,
    })
}

pub(super) fn resolve_interface_instance(
    mem: &mut MemReaderMemory,
    module: &ModuleImage,
    file_bytes: &[u8],
    interface_name: &str,
) -> Result<Option<usize>> {
    let entries = read_interfaces(mem, module, file_bytes)?;
    let Some(entry) = entries
        .into_iter()
        .find(|entry| entry.name.starts_with(interface_name))
    else {
        return Ok(None);
    };
    decode_interface_instance_addr(module, file_bytes, &entry)
}

pub(super) fn read_instance_class_name(
    mem: &mut MemReaderMemory,
    instance_addr: usize,
) -> Result<Option<String>> {
    let vtable = mem.read_u64(instance_addr)? as usize;
    if vtable < 8 {
        return Ok(None);
    }
    let typeinfo = mem.read_u64(vtable - 8)? as usize;
    if typeinfo == 0 {
        return Ok(None);
    }
    let name_ptr = mem.read_u64(typeinfo + 8)? as usize;
    if name_ptr == 0 {
        return Ok(None);
    }
    Ok(Some(mem.read_c_string(name_ptr, 128)?))
}

fn read_interfaces(
    mem: &mut MemReaderMemory,
    module: &ModuleImage,
    file_bytes: &[u8],
) -> Result<Vec<InterfaceEntry>> {
    let create_interface_virtual =
        find_symbol_virtual_addr(file_bytes, CREATE_INTERFACE_SYMBOL)?.map(|value| value as usize);
    let interface_list_virtual = find_interface_list_virtual(file_bytes, create_interface_virtual)?
        .context("failed to discover InterfaceReg list")?;
    let interface_list_addr = module.image_base + interface_list_virtual;
    let first_entry_addr = mem.read_u64(interface_list_addr)? as usize;
    read_interface_entries(mem, first_entry_addr, 256)
}

fn decode_interface_instance_addr(
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
    let code = file_bytes
        .get(file_offset..file_offset + 8)
        .context("create_fn offset is outside file bounds for interface instance decode")?;
    if !code.starts_with(CREATE_FN_RET_PATTERN) || code.get(7).copied() != Some(0xC3) {
        return Ok(None);
    }

    let disp = i32::from_le_bytes([code[3], code[4], code[5], code[6]]) as i64;
    let instance_virtual = ((create_fn_virtual + 7) as i64 + disp) as usize;
    Ok(Some(module.image_base + instance_virtual))
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
            let window_end = (window_start + 0x40).min(file_bytes.len());
            let local_matches = find_matches(&file_bytes[window_start..window_end], &pattern, 1);
            if let Some(local) = local_matches.first() {
                let match_virtual = symbol_virtual + local;
                return Ok(Some(decode_rip_target(
                    match_virtual,
                    &file_bytes[window_start + local..],
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
    Ok(((virtual_match_offset + 7) as i64 + disp) as usize)
}

fn read_interface_entries(
    mem: &mut MemReaderMemory,
    mut cursor: usize,
    max_entries: usize,
) -> Result<Vec<InterfaceEntry>> {
    let mut entries = Vec::new();

    while cursor != 0 && entries.len() < max_entries {
        let create_fn = mem.read_u64(cursor)? as usize;
        let name_ptr = mem.read_u64(cursor + 0x8)? as usize;
        let next = mem.read_u64(cursor + 0x10)? as usize;
        let name = mem.read_c_string(name_ptr, 128)?;
        if name.is_empty() {
            break;
        }

        entries.push(InterfaceEntry { name, create_fn });
        cursor = next;
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::{ModuleImage, module_image};
    use memreader_client::MemReaderModuleInfo;

    #[test]
    fn module_image_prefers_file_offset_zero_for_image_base() {
        let modules = vec![
            MemReaderModuleInfo {
                base: 0x2000,
                end: 0x3000,
                file_offset: 0x1000,
                perms: "r-xp".to_string(),
                path: "/tmp/libclient.so".to_string(),
            },
            MemReaderModuleInfo {
                base: 0x1000,
                end: 0x2000,
                file_offset: 0,
                perms: "r-xp".to_string(),
                path: "/tmp/libclient.so".to_string(),
            },
        ];

        assert_eq!(
            module_image(&modules, "libclient.so"),
            Some(ModuleImage {
                path: "/tmp/libclient.so".to_string(),
                image_base: 0x1000,
                image_end: 0x3000,
            })
        );
    }
}
