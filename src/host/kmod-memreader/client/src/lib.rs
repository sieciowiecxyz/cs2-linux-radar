use std::ffi::CString;
use std::mem;
use std::os::fd::RawFd;

use anyhow::{Result, anyhow, bail};
use libc::{MAP_SHARED, O_CLOEXEC, O_RDWR, POLLIN, c_void, mmap, munmap, open, poll, pollfd};
pub use memreader_uapi::TargetSelector;

use memreader_uapi::{
    AdvanceTailRequest, GetInfoResponse, InspectHostProcessRequest, ListModulesRequest,
    MEMREADER_DEVICE_NAME, MEMREADER_IOCTL_ADVANCE_TAIL, MEMREADER_IOCTL_GET_INFO,
    MEMREADER_IOCTL_INSPECT_HOST_PROCESS, MEMREADER_IOCTL_LIST_MODULES,
    MEMREADER_IOCTL_RESOLVE_HOST_PROCESS, MEMREADER_IOCTL_SUBMIT_READ,
    MEMREADER_IOCTL_UPSERT_TARGET, MEMREADER_RECORD_KIND_DATA, MEMREADER_RECORD_KIND_PADDING,
    MEMREADER_TASK_COMM_BYTES, MEMREADER_UAPI_VERSION, MemReaderRangeResult, MemReaderRecordHeader,
    MemReaderRingLayout, ModuleEntry, ReadRange, RecordStatus, ResolveHostProcessRequest,
    SubmitReadRequest, UpsertTargetRequest,
};

pub const DEFAULT_TARGET_SLOT: u32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostProcessInfo {
    pub pid: u32,
    pub start_time_ticks: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostProcessDetails {
    pub pid: u32,
    pub process_name: String,
    pub start_time_ticks: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemReaderModuleInfo {
    pub base: usize,
    pub end: usize,
    pub file_offset: usize,
    pub perms: String,
    pub path: String,
}

pub struct MemReaderDevice {
    fd: RawFd,
    mapping: *mut c_void,
    info: GetInfoResponse,
}

unsafe impl Send for MemReaderDevice {}

impl MemReaderDevice {
    pub fn open() -> Result<Self> {
        let (fd, opened_path) = open_first_available_device()?;

        let mut info = GetInfoResponse::default();
        let rc = unsafe { libc::ioctl(fd, MEMREADER_IOCTL_GET_INFO as _, &mut info) };
        if rc < 0 {
            unsafe { libc::close(fd) };
            return Err(anyhow!(
                "ioctl(GET_INFO) on {opened_path} failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        if info.uapi_version != MEMREADER_UAPI_VERSION {
            unsafe { libc::close(fd) };
            bail!(
                "uapi version mismatch on {opened_path}: kernel={} userspace={}",
                info.uapi_version,
                MEMREADER_UAPI_VERSION
            );
        }

        let mapping = unsafe {
            mmap(
                core::ptr::null_mut(),
                info.ring_mapping_bytes as usize,
                libc::PROT_READ,
                MAP_SHARED,
                fd,
                0,
            )
        };
        if mapping == libc::MAP_FAILED {
            unsafe { libc::close(fd) };
            return Err(anyhow!(
                "mmap failed on {opened_path}: {}",
                std::io::Error::last_os_error()
            ));
        }

        Ok(Self { fd, mapping, info })
    }

    pub fn info(&self) -> GetInfoResponse {
        self.info
    }

    pub fn upsert_target(&self, slot: u32, selector: TargetSelector) -> Result<()> {
        let req = UpsertTargetRequest {
            slot,
            flags: 0,
            selector,
        };
        let rc = unsafe { libc::ioctl(self.fd, MEMREADER_IOCTL_UPSERT_TARGET as _, &req) };
        if rc < 0 {
            return Err(anyhow!(
                "ioctl(UPSERT_TARGET) failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    pub fn resolve_host_process_by_name(&self, process_name: &str) -> Result<HostProcessInfo> {
        let mut req = ResolveHostProcessRequest::default();
        write_comm_name(&mut req.process_name, process_name)?;
        let rc =
            unsafe { libc::ioctl(self.fd, MEMREADER_IOCTL_RESOLVE_HOST_PROCESS as _, &mut req) };
        if rc < 0 {
            return Err(anyhow!(
                "ioctl(RESOLVE_HOST_PROCESS) failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        match req.match_count {
            0 => bail!("could not find process `{process_name}` via memreader"),
            1 => Ok(HostProcessInfo {
                pid: req.pid,
                start_time_ticks: req.start_time_ticks,
            }),
            _ => {
                bail!(
                    "found multiple `{process_name}` processes via memreader; rerun with explicit pid"
                )
            }
        }
    }

    pub fn inspect_host_process(&self, pid: u32) -> Result<HostProcessDetails> {
        let mut req = InspectHostProcessRequest {
            pid,
            ..InspectHostProcessRequest::default()
        };
        let rc =
            unsafe { libc::ioctl(self.fd, MEMREADER_IOCTL_INSPECT_HOST_PROCESS as _, &mut req) };
        if rc < 0 {
            return Err(anyhow!(
                "ioctl(INSPECT_HOST_PROCESS) failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        Ok(HostProcessDetails {
            pid,
            process_name: decode_fixed_string(&req.process_name),
            start_time_ticks: req.start_time_ticks,
        })
    }

    pub fn list_modules(&self, selector: TargetSelector) -> Result<Vec<MemReaderModuleInfo>> {
        let mut capacity = 64usize;
        loop {
            let mut entries = vec![ModuleEntry::default(); capacity];
            let mut req = ListModulesRequest {
                selector,
                entries_ptr: entries.as_mut_ptr() as u64,
                capacity: entries.len() as u32,
                ..ListModulesRequest::default()
            };
            let rc = unsafe { libc::ioctl(self.fd, MEMREADER_IOCTL_LIST_MODULES as _, &mut req) };
            if rc < 0 {
                return Err(anyhow!(
                    "ioctl(LIST_MODULES) failed: {}",
                    std::io::Error::last_os_error()
                ));
            }

            if (req.total_matches as usize) > capacity {
                capacity = req.total_matches as usize;
                continue;
            }

            entries.truncate(req.returned as usize);
            return entries
                .into_iter()
                .map(|entry| {
                    Ok(MemReaderModuleInfo {
                        base: entry.base as usize,
                        end: entry.end as usize,
                        file_offset: entry.file_offset as usize,
                        perms: decode_perms(&entry.perms),
                        path: decode_module_path(&entry)?,
                    })
                })
                .collect();
        }
    }

    pub fn submit_read(&self, slot: u32, cookie: u64, range: ReadRange) -> Result<RawRecord> {
        let mut req = SubmitReadRequest {
            target_slot: slot,
            flags: 0,
            cookie,
            range_count: 1,
            reserved0: 0,
            ..SubmitReadRequest::default()
        };
        req.ranges[0] = range;
        let rc = unsafe { libc::ioctl(self.fd, MEMREADER_IOCTL_SUBMIT_READ as _, &req) };
        if rc < 0 {
            return Err(anyhow!(
                "ioctl(SUBMIT_READ) failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        let mut pfd = pollfd {
            fd: self.fd,
            events: POLLIN,
            revents: 0,
        };
        let poll_rc = unsafe { poll(&mut pfd, 1, 2000) };
        if poll_rc <= 0 {
            bail!("poll() timed out or failed");
        }

        let layout = unsafe { &*(self.mapping.cast::<MemReaderRingLayout>()) };
        let record = decode_next_record_raw(layout, self.info.ring_mapping_bytes as usize)?;
        self.advance_tail(record.next_consumer_tail)?;
        Ok(record)
    }

    fn advance_tail(&self, consumer_tail: u64) -> Result<()> {
        let req = AdvanceTailRequest { consumer_tail };
        let rc = unsafe { libc::ioctl(self.fd, MEMREADER_IOCTL_ADVANCE_TAIL as _, &req) };
        if rc < 0 {
            return Err(anyhow!(
                "ioctl(ADVANCE_TAIL) failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    pub fn read_bytes(
        &self,
        slot: u32,
        cookie: u64,
        remote_addr: usize,
        len: u32,
    ) -> Result<Vec<u8>> {
        let snapshot = self.submit_read(
            slot,
            cookie,
            ReadRange {
                remote_addr: remote_addr as u64,
                len,
                reserved0: 0,
            },
        )?;
        if snapshot.header.status != RecordStatus::Ok as u32 {
            bail!(
                "snapshot read failed: status={} ({})",
                snapshot.header.status,
                record_status_name(snapshot.header.status)
            );
        }
        let bytes = snapshot.payload;
        anyhow::ensure!(
            bytes.len() == len as usize,
            "payload length mismatch: expected {}, got {}",
            len,
            bytes.len()
        );
        Ok(bytes)
    }

    pub fn read_u64(&self, slot: u32, cookie: u64, remote_addr: usize) -> Result<u64> {
        let mut buf = [0; 8];
        let bytes = self.read_bytes(slot, cookie, remote_addr, mem::size_of::<u64>() as u32)?;
        buf.copy_from_slice(&bytes);
        Ok(u64::from_le_bytes(buf))
    }

    pub fn open_target(
        self,
        slot: u32,
        selector: TargetSelector,
    ) -> Result<MemReaderTargetSession> {
        self.upsert_target(slot, selector)?;
        Ok(MemReaderTargetSession {
            device: self,
            slot,
            next_cookie: 1,
        })
    }
}

fn open_first_available_device() -> Result<(RawFd, &'static str)> {
    let c_path = CString::new(MEMREADER_DEVICE_NAME)?;
    let fd = unsafe { open(c_path.as_ptr(), O_RDWR | O_CLOEXEC) };
    if fd >= 0 {
        return Ok((fd, MEMREADER_DEVICE_NAME));
    }

    Err(anyhow!(
        "open({MEMREADER_DEVICE_NAME}) failed: {}",
        std::io::Error::last_os_error()
    ))
}

impl Drop for MemReaderDevice {
    fn drop(&mut self) {
        unsafe {
            munmap(self.mapping, self.info.ring_mapping_bytes as usize);
            libc::close(self.fd);
        }
    }
}

pub struct MemReaderTargetSession {
    device: MemReaderDevice,
    slot: u32,
    next_cookie: u64,
}

unsafe impl Send for MemReaderTargetSession {}

impl MemReaderTargetSession {
    pub fn open(slot: u32, selector: TargetSelector) -> Result<Self> {
        MemReaderDevice::open()?.open_target(slot, selector)
    }

    pub fn open_host_process(slot: u32, pid: u32, start_time_ticks: u64) -> Result<Self> {
        Self::open(slot, TargetSelector::host_pid(pid, start_time_ticks))
    }

    pub fn slot(&self) -> u32 {
        self.slot
    }

    pub fn read_raw(&mut self, remote_addr: usize, len: u32) -> Result<RawRecord> {
        let cookie = self.next_cookie;
        self.next_cookie = self.next_cookie.wrapping_add(1);
        self.device.submit_read(
            self.slot,
            cookie,
            ReadRange {
                remote_addr: remote_addr as u64,
                len,
                reserved0: 0,
            },
        )
    }

    pub fn read_bytes(&mut self, remote_addr: usize, len: u32) -> Result<Vec<u8>> {
        let snapshot = self.read_raw(remote_addr, len)?;
        if snapshot.header.status != RecordStatus::Ok as u32 {
            bail!(
                "snapshot read failed: status={} ({})",
                snapshot.header.status,
                record_status_name(snapshot.header.status)
            );
        }
        let bytes = snapshot.payload;
        anyhow::ensure!(
            bytes.len() == len as usize,
            "payload length mismatch: expected {}, got {}",
            len,
            bytes.len()
        );
        Ok(bytes)
    }

    pub fn read_exact_at(&mut self, address: usize, buf: &mut [u8]) -> Result<()> {
        let bytes = self.read_bytes(address, buf.len() as u32)?;
        buf.copy_from_slice(&bytes);
        Ok(())
    }

    pub fn read_u64(&mut self, address: usize) -> Result<u64> {
        let mut buf = [0; 8];
        self.read_exact_at(address, &mut buf)?;
        Ok(u64::from_le_bytes(buf))
    }

    pub fn read_i32(&mut self, address: usize) -> Result<i32> {
        let mut buf = [0; 4];
        self.read_exact_at(address, &mut buf)?;
        Ok(i32::from_le_bytes(buf))
    }

    pub fn read_u32(&mut self, address: usize) -> Result<u32> {
        let mut buf = [0; 4];
        self.read_exact_at(address, &mut buf)?;
        Ok(u32::from_le_bytes(buf))
    }

    pub fn read_u8(&mut self, address: usize) -> Result<u8> {
        let mut buf = [0; 1];
        self.read_exact_at(address, &mut buf)?;
        Ok(buf[0])
    }

    pub fn read_i16(&mut self, address: usize) -> Result<i16> {
        let mut buf = [0; 2];
        self.read_exact_at(address, &mut buf)?;
        Ok(i16::from_le_bytes(buf))
    }

    pub fn read_f32(&mut self, address: usize) -> Result<f32> {
        let mut buf = [0; 4];
        self.read_exact_at(address, &mut buf)?;
        Ok(f32::from_le_bytes(buf))
    }

    pub fn read_c_string(&mut self, address: usize, max_len: usize) -> Result<String> {
        if max_len == 0 {
            bail!("read_c_string requires max_len > 0");
        }
        let bytes = self.read_bytes(address, max_len as u32)?;
        decode_c_string(&bytes)
    }
}

#[derive(Debug, Clone)]
pub struct RawRecord {
    pub header: MemReaderRecordHeader,
    pub ranges: Vec<MemReaderRangeResult>,
    pub payload: Vec<u8>,
    pub next_consumer_tail: u64,
}

pub fn record_status_name(status: u32) -> &'static str {
    match status {
        x if x == RecordStatus::Ok as u32 => "ok",
        x if x == RecordStatus::NoTask as u32 => "no_task",
        x if x == RecordStatus::StartTimeMismatch as u32 => "start_time_mismatch",
        x if x == RecordStatus::NoMm as u32 => "no_mm",
        x if x == RecordStatus::PartialRead as u32 => "partial_read",
        x if x == RecordStatus::RingFull as u32 => "ring_full",
        x if x == RecordStatus::BadTargetSlot as u32 => "bad_target_slot",
        x if x == RecordStatus::BadRequest as u32 => "bad_request",
        _ => "internal",
    }
}

pub fn decode_c_string(raw: &[u8]) -> Result<String> {
    let end = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
    Ok(String::from_utf8_lossy(&raw[..end]).to_string())
}

pub fn read_process_start_time_ticks(pid: u32) -> Result<u64> {
    Ok(MemReaderDevice::open()?
        .inspect_host_process(pid)?
        .start_time_ticks)
}

pub fn resolve_host_process_by_name(process_name: &str) -> Result<HostProcessInfo> {
    MemReaderDevice::open()?.resolve_host_process_by_name(process_name)
}

pub fn inspect_host_process(pid: u32) -> Result<HostProcessDetails> {
    MemReaderDevice::open()?.inspect_host_process(pid)
}

fn decode_next_record_raw(layout: &MemReaderRingLayout, mapping_bytes: usize) -> Result<RawRecord> {
    let layout_bytes = mem::size_of::<MemReaderRingLayout>();
    let header_bytes = mem::size_of::<MemReaderRecordHeader>();
    let range_bytes = mem::size_of::<MemReaderRangeResult>();
    if mapping_bytes < layout_bytes {
        bail!("corrupt ring: mapping shorter than layout header");
    }
    let capacity = layout.capacity_bytes as usize;
    if capacity == 0 {
        bail!("corrupt ring: zero capacity");
    }
    if capacity > mapping_bytes - layout_bytes {
        bail!(
            "corrupt ring: capacity {} exceeds mapped data bytes {}",
            capacity,
            mapping_bytes - layout_bytes
        );
    }
    let mut tail = layout.consumer_tail as usize;
    let head = layout.producer_head as usize;
    if tail >= capacity || head >= capacity {
        bail!("corrupt ring: head/tail out of bounds head={head} tail={tail} capacity={capacity}");
    }
    if tail == head {
        bail!("ring is empty");
    }

    let data = unsafe {
        (layout as *const MemReaderRingLayout)
            .cast::<u8>()
            .add(mem::size_of::<MemReaderRingLayout>())
    };
    loop {
        let contiguous = capacity - tail;
        if contiguous < header_bytes {
            tail = 0;
            if tail == head {
                bail!("ring is empty");
            }
            continue;
        }
        let header = unsafe { &*(data.add(tail).cast::<MemReaderRecordHeader>()) };
        let total_len = header.total_len as usize;
        if total_len == 0 {
            bail!("corrupt ring: zero-length record at tail {tail}");
        }
        if total_len > contiguous {
            bail!(
                "corrupt ring: record length {total_len} exceeds contiguous bytes {contiguous} at tail {tail}"
            );
        }
        let next_tail = (tail + total_len) % capacity;
        if next_tail == tail {
            bail!("corrupt ring: record at tail {tail} does not advance");
        }
        if header.kind == MEMREADER_RECORD_KIND_PADDING {
            if total_len < header_bytes {
                bail!(
                    "corrupt ring: padding length {total_len} shorter than header {header_bytes}"
                );
            }
            tail = next_tail;
            continue;
        }
        if header.kind != MEMREADER_RECORD_KIND_DATA {
            bail!("unexpected record kind {}", header.kind);
        }
        let range_count = header.range_count as usize;
        if range_count > memreader_uapi::MEMREADER_MAX_RANGES_PER_JOB {
            bail!(
                "corrupt ring: range_count {} exceeds max {}",
                range_count,
                memreader_uapi::MEMREADER_MAX_RANGES_PER_JOB
            );
        }
        let payload_bytes = header.payload_bytes as usize;
        if payload_bytes > memreader_uapi::MEMREADER_MAX_PAYLOAD_BYTES {
            bail!(
                "corrupt ring: payload_bytes {} exceeds max {}",
                payload_bytes,
                memreader_uapi::MEMREADER_MAX_PAYLOAD_BYTES
            );
        }
        let ranges_len = range_bytes
            .checked_mul(range_count)
            .ok_or_else(|| anyhow!("corrupt ring: range result length overflow"))?;
        let used_len = header_bytes
            .checked_add(ranges_len)
            .and_then(|value| value.checked_add(payload_bytes))
            .ok_or_else(|| anyhow!("corrupt ring: record length overflow"))?;
        if used_len > total_len {
            bail!("corrupt ring: record body {used_len} exceeds total_len {total_len}");
        }

        let ranges_ptr = unsafe { data.add(tail + header_bytes) };
        let ranges = unsafe {
            std::slice::from_raw_parts(ranges_ptr.cast::<MemReaderRangeResult>(), range_count)
        };
        let payload_ptr = unsafe { ranges_ptr.add(ranges_len) };
        let payload = unsafe { std::slice::from_raw_parts(payload_ptr, payload_bytes) };
        let next_consumer_tail = next_tail as u64;
        return Ok(RawRecord {
            header: *header,
            ranges: ranges.to_vec(),
            payload: payload.to_vec(),
            next_consumer_tail,
        });
    }
}

fn write_comm_name(out: &mut [u8; MEMREADER_TASK_COMM_BYTES], process_name: &str) -> Result<()> {
    if process_name.is_empty() {
        bail!("process name must not be empty");
    }
    if process_name.len() >= MEMREADER_TASK_COMM_BYTES {
        bail!(
            "process name `{process_name}` exceeds memreader comm limit {}",
            MEMREADER_TASK_COMM_BYTES - 1
        );
    }
    out.fill(0);
    out[..process_name.len()].copy_from_slice(process_name.as_bytes());
    Ok(())
}

fn decode_fixed_string(raw: &[u8]) -> String {
    let end = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end]).to_string()
}

fn decode_module_path(entry: &ModuleEntry) -> Result<String> {
    let len = (entry.path_len as usize).min(entry.path.len());
    decode_c_string(&entry.path[..len])
}

fn decode_perms(raw: &[u8; 4]) -> String {
    String::from_utf8_lossy(raw).to_string()
}

#[cfg(test)]
mod tests {
    use super::{MemReaderRecordHeader, MemReaderRingLayout, decode_next_record_raw};
    use memreader_uapi::{
        MEMREADER_MAX_RANGES_PER_JOB, MEMREADER_RECORD_KIND_DATA, MEMREADER_RECORD_KIND_PADDING,
    };

    fn ring_with_record(
        capacity: usize,
        tail: usize,
        head: usize,
        header: MemReaderRecordHeader,
    ) -> Vec<u8> {
        let header_bytes = std::mem::size_of::<MemReaderRingLayout>();
        let mut bytes = vec![0_u8; header_bytes + capacity];
        let layout = bytes.as_mut_ptr().cast::<MemReaderRingLayout>();

        unsafe {
            (*layout).capacity_bytes = capacity as u32;
            (*layout).producer_head = head as u64;
            (*layout).consumer_tail = tail as u64;

            if tail < capacity {
                let record = bytes
                    .as_mut_ptr()
                    .add(header_bytes + tail)
                    .cast::<MemReaderRecordHeader>();
                *record = header;
            }
        }
        bytes
    }

    fn decode(bytes: &[u8]) -> anyhow::Result<super::RawRecord> {
        let layout = bytes.as_ptr().cast::<MemReaderRingLayout>();
        decode_next_record_raw(unsafe { &*layout }, bytes.len())
    }

    #[test]
    fn decode_record_implicitly_wraps_when_tail_has_no_room_for_header() {
        let mut bytes = ring_with_record(
            128,
            0,
            56,
            MemReaderRecordHeader {
                total_len: 56,
                kind: MEMREADER_RECORD_KIND_DATA,
                ..MemReaderRecordHeader::default()
            },
        );
        let header_bytes = std::mem::size_of::<MemReaderRingLayout>();
        let layout = bytes.as_mut_ptr().cast::<MemReaderRingLayout>();
        unsafe {
            (*layout).consumer_tail = 120;
        }

        let record = decode(&bytes).expect("record should decode");
        assert_eq!(record.header.kind, MEMREADER_RECORD_KIND_DATA);
        assert_eq!(record.next_consumer_tail, 56);
        assert_eq!(bytes.len(), header_bytes + 128);
    }

    #[test]
    fn decode_rejects_head_or_tail_outside_capacity() {
        let header = MemReaderRecordHeader {
            total_len: 56,
            kind: MEMREADER_RECORD_KIND_DATA,
            ..MemReaderRecordHeader::default()
        };
        let bytes = ring_with_record(128, 128, 56, header);

        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn decode_rejects_zero_length_padding() {
        let bytes = ring_with_record(
            128,
            0,
            56,
            MemReaderRecordHeader {
                total_len: 0,
                kind: MEMREADER_RECORD_KIND_PADDING,
                ..MemReaderRecordHeader::default()
            },
        );

        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn decode_rejects_record_shorter_than_header() {
        let bytes = ring_with_record(
            128,
            0,
            8,
            MemReaderRecordHeader {
                total_len: 8,
                kind: MEMREADER_RECORD_KIND_DATA,
                ..MemReaderRecordHeader::default()
            },
        );

        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn decode_rejects_range_count_above_uapi_limit() {
        let bytes = ring_with_record(
            512,
            0,
            56,
            MemReaderRecordHeader {
                total_len: 56,
                kind: MEMREADER_RECORD_KIND_DATA,
                range_count: (MEMREADER_MAX_RANGES_PER_JOB + 1) as u32,
                ..MemReaderRecordHeader::default()
            },
        );

        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn decode_rejects_payload_outside_record_body() {
        let bytes = ring_with_record(
            128,
            0,
            56,
            MemReaderRecordHeader {
                total_len: 56,
                kind: MEMREADER_RECORD_KIND_DATA,
                payload_bytes: 1,
                ..MemReaderRecordHeader::default()
            },
        );

        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn decode_rejects_record_that_does_not_advance_tail() {
        let bytes = ring_with_record(
            128,
            0,
            56,
            MemReaderRecordHeader {
                total_len: 128,
                kind: MEMREADER_RECORD_KIND_PADDING,
                ..MemReaderRecordHeader::default()
            },
        );

        assert!(decode(&bytes).is_err());
    }
}
