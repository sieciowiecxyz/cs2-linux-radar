#![allow(missing_docs)]
#![allow(unreachable_pub)]

use core::{ffi::c_int, mem, ptr};

#[allow(dead_code)]
mod uapi {
    include!("../../uapi/src/layout.rs");
}

use uapi::{
    MEMREADER_MAX_TARGETS, MEMREADER_RECORD_KIND_DATA, MEMREADER_RECORD_KIND_PADDING,
    MEMREADER_UAPI_VERSION, MemReaderRangeResult, MemReaderRecordHeader, RangeReadStatus,
    RecordStatus,
};

#[repr(C)]
pub struct ThubTargetSlot {
    pub in_use: u8,
    pub reserved: [u8; 7],
    pub selector: uapi::TargetSelector,
}

#[repr(C)]
pub struct ThubSession {
    pub layout: *mut uapi::MemReaderRingLayout,
    pub mapping: *mut core::ffi::c_void,
    pub mapping_bytes: usize,
    pub waitq: [u8; 24],
    pub scratch: *mut u8,
    pub scratch_len: usize,
    pub slots: [ThubTargetSlot; MEMREADER_MAX_TARGETS],
    pub ring_capacity_bytes: usize,
    pub producer_head: u64,
    pub consumer_tail: u64,
    pub dropped_records: u64,
    pub next_seq: u64,
}

unsafe extern "C" {
    fn thub_kernel_wake_consumer(session: *mut ThubSession);
}

const fn align_up(value: usize, align: usize) -> usize {
    (value + (align - 1)) & !(align - 1)
}

const fn layout_header_bytes() -> usize {
    mem::size_of::<uapi::MemReaderRingLayout>()
}

const fn record_header_bytes() -> usize {
    mem::size_of::<MemReaderRecordHeader>()
}

const fn range_result_bytes(range_count: usize) -> usize {
    mem::size_of::<MemReaderRangeResult>() * range_count
}

const fn record_total_len(range_count: usize, payload_bytes: usize) -> usize {
    align_up(
        record_header_bytes() + range_result_bytes(range_count) + payload_bytes,
        8,
    )
}

unsafe fn ring_data_ptr(layout: *mut uapi::MemReaderRingLayout) -> *mut u8 {
    unsafe { layout.cast::<u8>().add(layout_header_bytes()) }
}

unsafe fn init_ring_layout(layout: *mut uapi::MemReaderRingLayout, mapping_bytes: usize) {
    unsafe {
        ptr::write_bytes(layout.cast::<u8>(), 0, layout_header_bytes());
        (*layout).uapi_version = MEMREADER_UAPI_VERSION;
        (*layout).header_bytes = layout_header_bytes() as u32;
        (*layout).capacity_bytes = mapping_bytes.saturating_sub(layout_header_bytes()) as u32;
    }
}

unsafe fn sync_ring_layout(session: *mut ThubSession) {
    unsafe {
        let layout = (*session).layout;
        if layout.is_null() {
            return;
        }
        (*layout).capacity_bytes = (*session).ring_capacity_bytes as u32;
        (*layout).producer_head = (*session).producer_head;
        (*layout).consumer_tail = (*session).consumer_tail;
        (*layout).dropped_records = (*session).dropped_records;
        (*layout).next_seq = (*session).next_seq;
    }
}

unsafe fn push_padding(session: *mut ThubSession, padding_len: usize) {
    unsafe {
        let layout = (*session).layout;
        let capacity = (*session).ring_capacity_bytes;
        let head = (*session).producer_head as usize;
        let data = ring_data_ptr(layout);
        let header = data.add(head).cast::<MemReaderRecordHeader>();
        ptr::write(
            header,
            MemReaderRecordHeader {
                total_len: padding_len as u32,
                kind: MEMREADER_RECORD_KIND_PADDING,
                ..MemReaderRecordHeader::default()
            },
        );
        (*session).producer_head = ((head + padding_len) % capacity) as u64;
        sync_ring_layout(session);
    }
}

unsafe fn push_record(
    session: *mut ThubSession,
    record_header: &MemReaderRecordHeader,
    range_results: *const MemReaderRangeResult,
    payload: *const u8,
) -> Result<(), ()> {
    let total_len = record_header.total_len as usize;
    let (capacity, head, tail) = unsafe {
        (
            (*session).ring_capacity_bytes,
            (*session).producer_head as usize,
            (*session).consumer_tail as usize,
        )
    };
    let wrap_overhead = if head + total_len > capacity {
        capacity.saturating_sub(head)
    } else {
        0
    };
    let free = if head >= tail {
        capacity.saturating_sub(head - tail)
    } else {
        tail - head
    };
    if total_len + wrap_overhead + 8 > free {
        unsafe {
            (*session).dropped_records = (*session).dropped_records.wrapping_add(1);
            sync_ring_layout(session);
        }
        return Err(());
    }

    let layout = unsafe { (*session).layout };
    let data = unsafe { ring_data_ptr(layout) };
    if head + total_len > capacity {
        let remaining = capacity - head;
        unsafe {
            if remaining >= record_header_bytes() {
                push_padding(session, remaining);
            } else {
                // The remaining tail bytes are too short to hold even a padding header.
                // Leave them unused and let the consumer perform an implicit wrap once
                // it sees fewer than `MemReaderRecordHeader` bytes to the end.
                ptr::write_bytes(data.add(head), 0, remaining);
                (*session).producer_head = 0;
                sync_ring_layout(session);
            }
        }
    }

    let head = unsafe { (*session).producer_head as usize };
    let base = unsafe { data.add(head) };
    unsafe {
        ptr::copy_nonoverlapping(
            (record_header as *const MemReaderRecordHeader).cast::<u8>(),
            base,
            record_header_bytes(),
        );
    }

    let ranges_len = range_result_bytes(record_header.range_count as usize);
    unsafe {
        ptr::copy_nonoverlapping(
            range_results.cast::<u8>(),
            base.add(record_header_bytes()),
            ranges_len,
        );
        ptr::copy_nonoverlapping(
            payload,
            base.add(record_header_bytes() + ranges_len),
            record_header.payload_bytes as usize,
        );
    }

    unsafe {
        (*session).producer_head = ((head + total_len) % capacity) as u64;
        (*session).next_seq = (*session).next_seq.wrapping_add(1);
        sync_ring_layout(session);
    }
    Ok(())
}

#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe extern "C" fn thub_rust_session_init(
    session: *mut ThubSession,
    mapping_bytes: usize,
) -> c_int {
    if session.is_null() || unsafe { (*session).layout.is_null() } {
        return -22;
    }
    unsafe {
        init_ring_layout((*session).layout, mapping_bytes);
        (*session).ring_capacity_bytes = mapping_bytes.saturating_sub(layout_header_bytes());
        (*session).producer_head = 0;
        (*session).consumer_tail = 0;
        (*session).dropped_records = 0;
        (*session).next_seq = 0;
        sync_ring_layout(session);
        for slot in &mut (*session).slots {
            ptr::write_bytes(
                (slot as *mut ThubTargetSlot).cast::<u8>(),
                0,
                mem::size_of::<ThubTargetSlot>(),
            );
        }
    }
    0
}

#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe extern "C" fn thub_rust_upsert_target(
    session: *mut ThubSession,
    req: *const uapi::UpsertTargetRequest,
) -> c_int {
    if session.is_null() || req.is_null() {
        return -22;
    }
    let req = unsafe { &*req };
    let Some(slot) = (unsafe { &mut (*session).slots }).get_mut(req.slot as usize) else {
        return -22;
    };
    slot.in_use = 1;
    slot.selector = req.selector;
    0
}

#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe extern "C" fn thub_rust_remove_target(
    session: *mut ThubSession,
    req: *const uapi::RemoveTargetRequest,
) -> c_int {
    if session.is_null() || req.is_null() {
        return -22;
    }
    let req = unsafe { &*req };
    let Some(slot) = (unsafe { &mut (*session).slots }).get_mut(req.slot as usize) else {
        return -22;
    };
    unsafe {
        ptr::write_bytes(
            (slot as *mut ThubTargetSlot).cast::<u8>(),
            0,
            mem::size_of::<ThubTargetSlot>(),
        );
    }
    0
}

#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe extern "C" fn thub_rust_publish_record(
    session: *mut ThubSession,
    req: *const uapi::SubmitReadRequest,
    status: u32,
    range_count: u32,
    range_results: *const MemReaderRangeResult,
    payload: *const u8,
    payload_bytes: usize,
) -> c_int {
    if session.is_null() || req.is_null() || range_results.is_null() || payload.is_null() {
        return -22;
    }
    if unsafe { (*session).layout.is_null() } {
        return -22;
    }
    let req = unsafe { &*req };
    let total_len = record_total_len(range_count as usize, payload_bytes);
    let header = MemReaderRecordHeader {
        total_len: total_len as u32,
        kind: MEMREADER_RECORD_KIND_DATA,
        seq: unsafe { (*session).next_seq },
        timestamp_ns: 0,
        cookie: req.cookie,
        target_slot: req.target_slot,
        status,
        range_count,
        reserved0: 0,
        payload_bytes: payload_bytes as u32,
        reserved1: 0,
    };
    if unsafe { push_record(session, &header, range_results, payload) }.is_err() {
        return -(RecordStatus::RingFull as c_int);
    }
    unsafe {
        thub_kernel_wake_consumer(session);
    }
    0
}

const _: () = {
    assert!(mem::size_of::<uapi::MemReaderRingLayout>() == 80);
    assert!(mem::size_of::<uapi::MemReaderRecordHeader>() == 56);
    assert!(mem::size_of::<MemReaderRangeResult>() == 24);
    assert!(RangeReadStatus::Ok as u32 == 0);
};
