use anyhow::{Context, Result};
use object::{Object, ObjectSegment, ObjectSymbol};

pub(crate) fn file_offset_for_virtual_addr(bytes: &[u8], virtual_addr: u64) -> Result<Option<u64>> {
    let file = object::File::parse(bytes).context("failed to parse object file")?;

    for segment in file.segments() {
        let address = segment.address();
        let size = segment.size();
        if virtual_addr >= address && virtual_addr < address.saturating_add(size) {
            let offset_in_segment = virtual_addr - address;
            let (file_offset, file_size) = segment.file_range();
            if offset_in_segment < file_size {
                return Ok(Some(file_offset + offset_in_segment));
            }
        }
    }

    Ok(None)
}

pub(crate) fn find_symbol_virtual_addr(bytes: &[u8], symbol_name: &str) -> Result<Option<u64>> {
    let file = object::File::parse(bytes).context("failed to parse object file")?;
    for symbol in file.dynamic_symbols() {
        if symbol.name().ok() == Some(symbol_name) {
            return Ok(Some(symbol.address()));
        }
    }
    for symbol in file.symbols() {
        if symbol.name().ok() == Some(symbol_name) {
            return Ok(Some(symbol.address()));
        }
    }
    Ok(None)
}
