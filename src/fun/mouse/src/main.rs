#![forbid(unsafe_code)]

mod app;
mod audio;
mod config;
mod device_discovery;
mod keyboard_monitor;
mod kill_switch;
mod mouse_grab;
mod relay;
mod trigger_client;
mod virtual_mouse;

fn main() -> anyhow::Result<()> {
    let _ = shared_logging::init("info");
    match config::Command::from_env_args()? {
        config::Command::Run(config) => app::run(config),
        config::Command::LearnBinding(config) => app::learn_binding(config),
        config::Command::Setup(config) => app::setup(config),
    }
}
