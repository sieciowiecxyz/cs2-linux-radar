use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use evdev::KeyCode;
use tracing::{error, info, warn};

use crate::audio::{self, ToggleSound};
use crate::config::{
    LearnBindingConfig, LearnedBinding, PersistedConfig, RunConfig, SetupConfig,
    parse_mouse_button_key,
};
use crate::device_discovery::{discover_keyboard_paths, discover_mouse_paths};
use crate::keyboard_monitor::{self, KeyboardCommand};
use crate::kill_switch::KillSwitch;
use crate::mouse_grab::PhysicalMouseSet;
use crate::relay::ButtonBindingState;
use crate::trigger_client::TriggerDecision;
use crate::virtual_mouse::VirtualMouse;

pub fn run(config: RunConfig) -> Result<()> {
    let persisted = PersistedConfig::load_or_default(&config.config_path)
        .with_context(|| format!("load {}", config.config_path.display()))?;
    let trigger_binding = persisted
        .bindings
        .trigger
        .as_deref()
        .map(parse_mouse_button_key)
        .transpose()
        .context("parse trigger binding from config")?;
    let mouse_paths = discover_mouse_paths().context("discover physical mice")?;
    let keyboard_paths =
        discover_keyboard_paths(KeyCode::KEY_F9).context("discover keyboard toggle devices")?;

    let mut mice = PhysicalMouseSet::open(&mouse_paths).context("open physical mice")?;
    mice.grab_all().context("initial grab of physical mice")?;

    let mut virtual_mouse =
        VirtualMouse::create(&config.device_name).context("create virtual mouse")?;
    let (tx, rx) = mpsc::channel();
    let _keyboard_thread = keyboard_monitor::spawn(keyboard_paths, KeyCode::KEY_F9, tx)
        .context("spawn keyboard monitor")?;

    let mut kill_switch = KillSwitch::new_enabled();
    let trigger_decision = if trigger_binding.is_some() {
        TriggerDecision::spawn()
    } else {
        warn!(
            config_path = %config.config_path.display(),
            "fun-mouse has no trigger binding configured; synthetic fire is disabled"
        );
        TriggerDecision::disabled()
    };
    let mut trigger_binding_state = trigger_binding.map(ButtonBindingState::new);
    let mut last_trigger_binding_down = false;
    let mut last_should_fire = false;
    info!(
        device_name = %config.device_name,
        trigger_binding = ?trigger_binding,
        "fun-mouse started"
    );

    loop {
        handle_commands(
            &rx,
            &mut kill_switch,
            &mut mice,
            &mut trigger_binding_state,
            &mut virtual_mouse,
        )?;

        let saw_events = mice.pump(
            kill_switch.enabled(),
            trigger_binding_state.as_mut(),
            &mut virtual_mouse,
        )?;
        let should_fire = kill_switch.enabled()
            && trigger_binding_state
                .as_ref()
                .is_some_and(ButtonBindingState::is_down)
            && trigger_decision.should_fire();
        let trigger_binding_down = trigger_binding_state
            .as_ref()
            .is_some_and(ButtonBindingState::is_down);
        if trigger_binding_down != last_trigger_binding_down {
            info!(
                trigger_binding = ?trigger_binding,
                state = if trigger_binding_down { "active" } else { "inactive" },
                "fun-mouse trigger hold state changed"
            );
            last_trigger_binding_down = trigger_binding_down;
        }
        if should_fire != last_should_fire {
            info!(should_fire, "fun-mouse synthetic trigger state changed");
            last_should_fire = should_fire;
        }
        virtual_mouse
            .set_synthetic_left(should_fire)
            .context("update synthetic trigger left button")?;
        if !saw_events {
            thread::sleep(Duration::from_millis(5));
        }
    }
}

pub fn learn_binding(config: LearnBindingConfig) -> Result<()> {
    let mouse_paths = discover_mouse_paths().context("discover physical mice")?;
    let mut mice = PhysicalMouseSet::open(&mouse_paths).context("open physical mice")?;
    mice.grab_all()
        .context("grab physical mice for learn-binding")?;

    let key_name = capture_binding(&mut mice, config.binding)?;
    let mut persisted = PersistedConfig::load_or_default(&config.config_path)
        .with_context(|| format!("load {}", config.config_path.display()))?;
    persisted.set_binding(config.binding, key_name.clone());
    persisted
        .save(&config.config_path)
        .with_context(|| format!("save {}", config.config_path.display()))?;

    println!(
        "Saved {} = {} to {}",
        config.binding.as_str(),
        key_name,
        config.config_path.display()
    );
    info!(
        binding = config.binding.as_str(),
        key = %key_name,
        config_path = %config.config_path.display(),
        "learned mouse binding"
    );
    Ok(())
}

pub fn setup(config: SetupConfig) -> Result<()> {
    let mouse_paths = discover_mouse_paths().context("discover physical mice")?;
    let mut mice = PhysicalMouseSet::open(&mouse_paths).context("open physical mice")?;
    mice.grab_all().context("grab physical mice for setup")?;

    let trigger = capture_binding(&mut mice, LearnedBinding::Trigger)?;

    let mut persisted = PersistedConfig::load_or_default(&config.config_path)
        .with_context(|| format!("load {}", config.config_path.display()))?;
    persisted.set_binding(LearnedBinding::Trigger, trigger.clone());
    persisted
        .save(&config.config_path)
        .with_context(|| format!("save {}", config.config_path.display()))?;

    println!("Saved trigger = {trigger}");
    println!("Config written to {}", config.config_path.display());
    info!(
        trigger = %trigger,
        config_path = %config.config_path.display(),
        "completed fun-mouse setup"
    );
    Ok(())
}

fn handle_commands(
    rx: &mpsc::Receiver<KeyboardCommand>,
    kill_switch: &mut KillSwitch,
    mice: &mut PhysicalMouseSet,
    trigger_binding_state: &mut Option<ButtonBindingState>,
    virtual_mouse: &mut VirtualMouse,
) -> Result<()> {
    while let Ok(command) = rx.try_recv() {
        match command {
            KeyboardCommand::ToggleRelay => {
                toggle_relay(kill_switch, mice, trigger_binding_state, virtual_mouse)?
            }
        }
    }
    Ok(())
}

fn toggle_relay(
    kill_switch: &mut KillSwitch,
    mice: &mut PhysicalMouseSet,
    trigger_binding_state: &mut Option<ButtonBindingState>,
    virtual_mouse: &mut VirtualMouse,
) -> Result<()> {
    if kill_switch.enabled() {
        virtual_mouse
            .release_pressed_buttons()
            .context("release pressed virtual mouse buttons before disable")?;
        if let Some(trigger_binding_state) = trigger_binding_state.as_mut() {
            trigger_binding_state.clear();
        }
        mice.ungrab_all();
        kill_switch.toggle();
        audio::play_toggle_async(ToggleSound::Disabled);
        info!("mouse relay disabled");
        return Ok(());
    }

    match mice.grab_all() {
        Ok(()) => {
            kill_switch.toggle();
            audio::play_toggle_async(ToggleSound::Enabled);
            info!("mouse relay enabled");
        }
        Err(err) => {
            kill_switch.disable();
            error!(error = %err, "failed to re-grab physical mice; relay stays disabled");
        }
    }

    Ok(())
}

fn capture_binding(mice: &mut PhysicalMouseSet, binding: LearnedBinding) -> Result<String> {
    println!(
        "Press the mouse button you want to use for {}...",
        binding.as_str()
    );
    info!(
        binding = binding.as_str(),
        "waiting for first mouse button press"
    );

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(key) = mice.poll_first_pressed_button()? {
            return Ok(format!("{key:?}"));
        }

        if Instant::now() >= deadline {
            bail!(
                "timed out waiting for a mouse button press for {}",
                binding.as_str()
            );
        }

        thread::sleep(Duration::from_millis(5));
    }
}
