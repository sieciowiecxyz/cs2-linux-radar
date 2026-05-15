use super::memory::{SnapshotMemory, read_string_ptr};

const NETWORK_CLIENT_SERVICE_INTERFACE: &str = "NetworkClientService_001";
const NETWORK_SERVICE_GAME_CLIENT_OFFSET: usize = 0xA0;
const NETWORK_GAME_CLIENT_MAP_PATH_OFFSET: usize = 0x220;
const NETWORK_GAME_CLIENT_MAP_NAME_OFFSET: usize = 0x228;

pub(super) fn read_map_name<B: SnapshotMemory>(runtime: &mut B) -> Option<String> {
    let service_addr = resolve_network_client_service_addr(runtime)?;
    let game_client_addr = runtime
        .read_u64(service_addr + NETWORK_SERVICE_GAME_CLIENT_OFFSET)
        .ok()? as usize;
    if game_client_addr == 0 {
        return None;
    }

    read_string_ptr(
        runtime,
        game_client_addr + NETWORK_GAME_CLIENT_MAP_NAME_OFFSET,
    )
    .or_else(|| {
        read_string_ptr(
            runtime,
            game_client_addr + NETWORK_GAME_CLIENT_MAP_PATH_OFFSET,
        )
    })
}

fn resolve_network_client_service_addr<B: SnapshotMemory>(runtime: &mut B) -> Option<usize> {
    runtime
        .resolve_interface_instance("libengine2.so", NETWORK_CLIENT_SERVICE_INTERFACE)
        .ok()?
}
