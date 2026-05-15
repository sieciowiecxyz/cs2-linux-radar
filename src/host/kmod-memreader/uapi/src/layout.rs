use core::mem;

pub const MEMREADER_UAPI_VERSION: u32 = 3;
pub const MEMREADER_DEVICE_NAME: &str = "/dev/memreader";
pub const MEMREADER_MAX_TARGETS: usize = 64;
pub const MEMREADER_MAX_RANGES_PER_JOB: usize = 32;
pub const MEMREADER_DEFAULT_RING_BYTES: usize = 16 * 1024 * 1024;
pub const MEMREADER_MAX_PAYLOAD_BYTES: usize = 1024 * 1024;
pub const MEMREADER_TASK_COMM_BYTES: usize = 16;
pub const MEMREADER_MODULE_PATH_BYTES: usize = 256;
pub const MEMREADER_RECORD_KIND_DATA: u32 = 1;
pub const MEMREADER_RECORD_KIND_PADDING: u32 = 2;

const IOC_NRBITS: u32 = 8;
const IOC_TYPEBITS: u32 = 8;
const IOC_SIZEBITS: u32 = 14;

const IOC_NRSHIFT: u32 = 0;
const IOC_TYPESHIFT: u32 = IOC_NRSHIFT + IOC_NRBITS;
const IOC_SIZESHIFT: u32 = IOC_TYPESHIFT + IOC_TYPEBITS;
const IOC_DIRSHIFT: u32 = IOC_SIZESHIFT + IOC_SIZEBITS;

const IOC_WRITE: u32 = 1;
const IOC_READ: u32 = 2;
const MEMREADER_IOC_MAGIC: u32 = b'T' as u32;

const fn ioc(dir: u32, ty: u32, nr: u32, size: u32) -> u64 {
    ((dir << IOC_DIRSHIFT) | (ty << IOC_TYPESHIFT) | (nr << IOC_NRSHIFT) | (size << IOC_SIZESHIFT))
        as u64
}

const fn ior<T>(ty: u32, nr: u32) -> u64 {
    ioc(IOC_READ, ty, nr, mem::size_of::<T>() as u32)
}

const fn iow<T>(ty: u32, nr: u32) -> u64 {
    ioc(IOC_WRITE, ty, nr, mem::size_of::<T>() as u32)
}

const fn iowr<T>(ty: u32, nr: u32) -> u64 {
    ioc(IOC_READ | IOC_WRITE, ty, nr, mem::size_of::<T>() as u32)
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetKind {
    Empty = 0,
    HostPid = 1,
    NamespacedPid = 2,
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RangeReadStatus {
    Ok = 0,
    Partial = 1,
    Fault = 2,
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordStatus {
    Ok = 0,
    NoTask = 1,
    StartTimeMismatch = 2,
    NoMm = 3,
    PartialRead = 4,
    RingFull = 5,
    BadTargetSlot = 6,
    BadRequest = 7,
    Internal = 8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TargetSelector {
    pub kind: u32,
    pub flags: u32,
    pub host_pid: u32,
    pub pid_in_ns: u32,
    pub container_init_host_pid: u32,
    pub reserved0: u32,
    pub start_time_ticks: u64,
}

impl TargetSelector {
    pub const fn host_pid(host_pid: u32, start_time_ticks: u64) -> Self {
        Self {
            kind: TargetKind::HostPid as u32,
            flags: 0,
            host_pid,
            pid_in_ns: 0,
            container_init_host_pid: 0,
            reserved0: 0,
            start_time_ticks,
        }
    }

    pub const fn namespaced(
        container_init_host_pid: u32,
        pid_in_ns: u32,
        start_time_ticks: u64,
    ) -> Self {
        Self {
            kind: TargetKind::NamespacedPid as u32,
            flags: 0,
            host_pid: 0,
            pid_in_ns,
            container_init_host_pid,
            reserved0: 0,
            start_time_ticks,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct UpsertTargetRequest {
    pub slot: u32,
    pub flags: u32,
    pub selector: TargetSelector,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RemoveTargetRequest {
    pub slot: u32,
    pub reserved0: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct AdvanceTailRequest {
    pub consumer_tail: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ReadRange {
    pub remote_addr: u64,
    pub len: u32,
    pub reserved0: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SubmitReadRequest {
    pub target_slot: u32,
    pub flags: u32,
    pub cookie: u64,
    pub range_count: u32,
    pub reserved0: u32,
    pub ranges: [ReadRange; MEMREADER_MAX_RANGES_PER_JOB],
}

impl Default for SubmitReadRequest {
    fn default() -> Self {
        Self {
            target_slot: 0,
            flags: 0,
            cookie: 0,
            range_count: 0,
            reserved0: 0,
            ranges: [ReadRange::default(); MEMREADER_MAX_RANGES_PER_JOB],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct GetInfoResponse {
    pub uapi_version: u32,
    pub ring_mapping_bytes: u32,
    pub max_targets: u32,
    pub max_ranges_per_job: u32,
    pub max_payload_bytes: u32,
    pub reserved0: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ResolveHostProcessRequest {
    pub process_name: [u8; MEMREADER_TASK_COMM_BYTES],
    pub match_count: u32,
    pub pid: u32,
    pub reserved0: u32,
    pub start_time_ticks: u64,
}

impl Default for ResolveHostProcessRequest {
    fn default() -> Self {
        Self {
            process_name: [0; MEMREADER_TASK_COMM_BYTES],
            match_count: 0,
            pid: 0,
            reserved0: 0,
            start_time_ticks: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct InspectHostProcessRequest {
    pub pid: u32,
    pub reserved0: u32,
    pub process_name: [u8; MEMREADER_TASK_COMM_BYTES],
    pub start_time_ticks: u64,
}

impl Default for InspectHostProcessRequest {
    fn default() -> Self {
        Self {
            pid: 0,
            reserved0: 0,
            process_name: [0; MEMREADER_TASK_COMM_BYTES],
            start_time_ticks: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ModuleEntry {
    pub base: u64,
    pub end: u64,
    pub file_offset: u64,
    pub perms: [u8; 4],
    pub path_len: u32,
    pub reserved0: u16,
    pub reserved1: u16,
    pub path: [u8; MEMREADER_MODULE_PATH_BYTES],
}

impl Default for ModuleEntry {
    fn default() -> Self {
        Self {
            base: 0,
            end: 0,
            file_offset: 0,
            perms: [0; 4],
            path_len: 0,
            reserved0: 0,
            reserved1: 0,
            path: [0; MEMREADER_MODULE_PATH_BYTES],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ListModulesRequest {
    pub selector: TargetSelector,
    pub entries_ptr: u64,
    pub capacity: u32,
    pub returned: u32,
    pub total_matches: u32,
    pub reserved0: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MemReaderRingLayout {
    pub uapi_version: u32,
    pub header_bytes: u32,
    pub capacity_bytes: u32,
    pub reserved0: u32,
    pub producer_head: u64,
    pub consumer_tail: u64,
    pub dropped_records: u64,
    pub next_seq: u64,
    pub reserved1: [u64; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MemReaderRangeResult {
    pub remote_addr: u64,
    pub requested_len: u32,
    pub bytes_read: u32,
    pub status: u32,
    pub reserved0: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MemReaderRecordHeader {
    pub total_len: u32,
    pub kind: u32,
    pub seq: u64,
    pub timestamp_ns: u64,
    pub cookie: u64,
    pub target_slot: u32,
    pub status: u32,
    pub range_count: u32,
    pub reserved0: u32,
    pub payload_bytes: u32,
    pub reserved1: u32,
}

pub const MEMREADER_IOCTL_GET_INFO: u64 = ior::<GetInfoResponse>(MEMREADER_IOC_MAGIC, 0x01);
pub const MEMREADER_IOCTL_UPSERT_TARGET: u64 =
    iow::<UpsertTargetRequest>(MEMREADER_IOC_MAGIC, 0x02);
pub const MEMREADER_IOCTL_REMOVE_TARGET: u64 =
    iow::<RemoveTargetRequest>(MEMREADER_IOC_MAGIC, 0x03);
pub const MEMREADER_IOCTL_SUBMIT_READ: u64 = iow::<SubmitReadRequest>(MEMREADER_IOC_MAGIC, 0x04);
pub const MEMREADER_IOCTL_ADVANCE_TAIL: u64 = iow::<AdvanceTailRequest>(MEMREADER_IOC_MAGIC, 0x05);
pub const MEMREADER_IOCTL_RESOLVE_HOST_PROCESS: u64 =
    iowr::<ResolveHostProcessRequest>(MEMREADER_IOC_MAGIC, 0x06);
pub const MEMREADER_IOCTL_INSPECT_HOST_PROCESS: u64 =
    iowr::<InspectHostProcessRequest>(MEMREADER_IOC_MAGIC, 0x07);
pub const MEMREADER_IOCTL_LIST_MODULES: u64 = iowr::<ListModulesRequest>(MEMREADER_IOC_MAGIC, 0x08);
