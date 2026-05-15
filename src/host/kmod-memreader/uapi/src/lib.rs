#![no_std]

pub mod layout;
pub mod ring;

pub use layout::*;
pub use ring::*;

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use crate::{
        MEMREADER_MAX_RANGES_PER_JOB, MemReaderRecordHeader, MemReaderRingLayout, record_total_len,
    };
    use core::mem;

    #[test]
    fn record_layout_is_stable() {
        assert_eq!(mem::size_of::<MemReaderRingLayout>(), 80);
        assert_eq!(mem::size_of::<MemReaderRecordHeader>(), 56);
    }

    #[test]
    fn submit_request_range_count_matches_constant() {
        assert_eq!(MEMREADER_MAX_RANGES_PER_JOB, 32);
        assert_eq!(record_total_len(1, 64) % 8, 0);
    }
}
