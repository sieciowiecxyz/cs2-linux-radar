use std::collections::HashSet;

use anyhow::Result;
use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventSummary, InputEvent, KeyCode, RelativeAxisCode};
use tracing::info;

pub struct VirtualMouse {
    device: VirtualDevice,
    emitted_buttons: HashSet<KeyCode>,
    physical_left_down: bool,
    synthetic_left_down: bool,
}

impl VirtualMouse {
    pub fn create(name: &str) -> Result<Self> {
        let mut buttons = AttributeSet::<KeyCode>::new();
        for &btn in &[
            KeyCode::BTN_LEFT,
            KeyCode::BTN_RIGHT,
            KeyCode::BTN_MIDDLE,
            KeyCode::BTN_SIDE,
            KeyCode::BTN_EXTRA,
        ] {
            buttons.insert(btn);
        }

        let mut axes = AttributeSet::<RelativeAxisCode>::new();
        for &axis in &[
            RelativeAxisCode::REL_X,
            RelativeAxisCode::REL_Y,
            RelativeAxisCode::REL_WHEEL,
            RelativeAxisCode::REL_HWHEEL,
        ] {
            axes.insert(axis);
        }

        let mut device = VirtualDevice::builder()?
            .name(name)
            .with_keys(&buttons)?
            .with_relative_axes(&axes)?
            .build()?;

        for path in device.enumerate_dev_nodes_blocking()? {
            let path = path?;
            info!(path = %path.display(), name, "virtual mouse is available");
        }

        Ok(Self {
            device,
            emitted_buttons: HashSet::new(),
            physical_left_down: false,
            synthetic_left_down: false,
        })
    }

    pub fn emit_batch(&mut self, events: &[InputEvent]) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        let mut output = Vec::with_capacity(events.len() + 1);
        for event in events {
            match event.destructure() {
                EventSummary::Key(_, KeyCode::BTN_LEFT, value) if matches!(value, 0 | 1) => {
                    self.physical_left_down = value == 1;
                }
                EventSummary::Key(_, key, value) if is_supported_button(key) => {
                    self.update_emitted_button(key, value);
                    output.push(*event);
                }
                _ => output.push(*event),
            }
        }
        if let Some(left_edge) = self.sync_left_edge() {
            output.push(left_edge);
        }
        if !output.is_empty() {
            self.device.emit(&output)?;
        }
        Ok(())
    }

    pub fn set_synthetic_left(&mut self, down: bool) -> Result<()> {
        if self.synthetic_left_down == down {
            return Ok(());
        }
        self.synthetic_left_down = down;
        if let Some(left_edge) = self.sync_left_edge() {
            self.device.emit(&[left_edge])?;
        }
        Ok(())
    }

    pub fn release_pressed_buttons(&mut self) -> Result<()> {
        if self.emitted_buttons.is_empty() {
            self.physical_left_down = false;
            self.synthetic_left_down = false;
            return Ok(());
        }

        let releases = self
            .emitted_buttons
            .iter()
            .copied()
            .map(|button| InputEvent::new(evdev::EventType::KEY.0, button.0, 0))
            .collect::<Vec<_>>();
        self.emitted_buttons.clear();
        self.physical_left_down = false;
        self.synthetic_left_down = false;
        self.device.emit(&releases)?;
        Ok(())
    }

    fn update_emitted_button(&mut self, key: KeyCode, value: i32) {
        match value {
            1 => {
                self.emitted_buttons.insert(key);
            }
            0 => {
                self.emitted_buttons.remove(&key);
            }
            _ => {}
        }
    }

    fn sync_left_edge(&mut self) -> Option<InputEvent> {
        let desired_down = self.physical_left_down || self.synthetic_left_down;
        let emitted_down = self.emitted_buttons.contains(&KeyCode::BTN_LEFT);
        if desired_down == emitted_down {
            return None;
        }
        if desired_down {
            self.emitted_buttons.insert(KeyCode::BTN_LEFT);
        } else {
            self.emitted_buttons.remove(&KeyCode::BTN_LEFT);
        }
        Some(InputEvent::new(
            evdev::EventType::KEY.0,
            KeyCode::BTN_LEFT.0,
            i32::from(desired_down),
        ))
    }
}

fn is_supported_button(key: KeyCode) -> bool {
    matches!(
        key,
        KeyCode::BTN_LEFT
            | KeyCode::BTN_RIGHT
            | KeyCode::BTN_MIDDLE
            | KeyCode::BTN_SIDE
            | KeyCode::BTN_EXTRA
    )
}
