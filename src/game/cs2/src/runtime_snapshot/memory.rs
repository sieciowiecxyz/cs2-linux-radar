use anyhow::Result;
use memreader_client::MemReaderModuleInfo;

pub(crate) trait MemoryReader {
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
}

pub(crate) trait SnapshotMemory: MemoryReader {
    fn pid(&self) -> u32;
    fn mapped_modules(&self) -> Vec<MemReaderModuleInfo>;
    fn read_module_file(&self, suffix: &str) -> Result<Vec<u8>>;
    fn resolve_interface_instance(
        &mut self,
        module_suffix: &str,
        interface_name: &str,
    ) -> Result<Option<usize>>;
    fn read_instance_class_name(&mut self, instance_addr: usize) -> Result<Option<String>>;
}

pub(super) fn read_string_ptr<B: SnapshotMemory>(
    runtime: &mut B,
    ptr_slot_addr: usize,
) -> Option<String> {
    let value = runtime.read_u64(ptr_slot_addr).ok()? as usize;
    if value == 0 {
        return None;
    }
    read_c_string(runtime, value, 128).ok()
}

pub(super) fn read_c_string<B: SnapshotMemory>(
    runtime: &mut B,
    address: usize,
    max_len: usize,
) -> Result<String> {
    let mut buf = vec![0; max_len];
    runtime.read_exact_at(address, &mut buf)?;
    let end = buf.iter().position(|byte| *byte == 0).unwrap_or(buf.len());
    Ok(String::from_utf8_lossy(&buf[..end]).to_string())
}

pub(super) fn read_vec2<B: SnapshotMemory>(runtime: &mut B, address: usize) -> Result<[f32; 2]> {
    Ok([runtime.read_f32(address)?, runtime.read_f32(address + 4)?])
}

pub(super) fn read_vec3<B: SnapshotMemory>(runtime: &mut B, address: usize) -> Result<[f32; 3]> {
    Ok([
        runtime.read_f32(address)?,
        runtime.read_f32(address + 4)?,
        runtime.read_f32(address + 8)?,
    ])
}

pub(super) fn read_vec4<B: SnapshotMemory>(runtime: &mut B, address: usize) -> Result<[f32; 4]> {
    Ok([
        runtime.read_f32(address)?,
        runtime.read_f32(address + 4)?,
        runtime.read_f32(address + 8)?,
        runtime.read_f32(address + 12)?,
    ])
}
