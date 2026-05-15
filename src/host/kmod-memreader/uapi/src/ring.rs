use crate::layout::{MemReaderRangeResult, MemReaderRecordHeader, MemReaderRingLayout};
use core::mem;

pub const fn align_up(value: usize, align: usize) -> usize {
    (value + (align - 1)) & !(align - 1)
}

pub const fn record_total_len(range_count: usize, payload_bytes: usize) -> usize {
    align_up(
        mem::size_of::<MemReaderRecordHeader>()
            + mem::size_of::<MemReaderRangeResult>() * range_count
            + payload_bytes,
        8,
    )
}

pub const fn layout_header_bytes() -> usize {
    mem::size_of::<MemReaderRingLayout>()
}

pub const fn record_header_bytes() -> usize {
    mem::size_of::<MemReaderRecordHeader>()
}

pub const fn range_result_bytes(range_count: usize) -> usize {
    mem::size_of::<MemReaderRangeResult>() * range_count
}
