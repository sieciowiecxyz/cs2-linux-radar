use anyhow::Result;
use memreader_client::{MemReaderDevice, resolve_host_process_by_name};

pub(super) const PROCESS_NAME: &str = "cs2";

pub(super) fn resolve_pid(requested_pid: Option<u32>) -> Result<u32> {
    if let Some(pid) = requested_pid {
        ensure_process_name(pid, PROCESS_NAME)?;
        return Ok(pid);
    }

    Ok(resolve_host_process_by_name(PROCESS_NAME)?.pid)
}

pub(super) fn ensure_process_name(pid: u32, expected: &str) -> Result<()> {
    let actual = MemReaderDevice::open()?
        .inspect_host_process(pid)?
        .process_name;
    anyhow::ensure!(
        actual == expected,
        "pid {pid} is '{actual}', expected '{expected}'"
    );
    Ok(())
}

pub(super) fn inspect_process_start_time(pid: u32) -> Result<u64> {
    Ok(MemReaderDevice::open()?
        .inspect_host_process(pid)?
        .start_time_ticks)
}
