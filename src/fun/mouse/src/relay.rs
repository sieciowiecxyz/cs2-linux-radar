use anyhow::Result;
use evdev::{EventSummary, InputEvent, KeyCode, RelativeAxisCode, SynchronizationCode};

use crate::virtual_mouse::VirtualMouse;

#[derive(Debug, Clone, Copy)]
pub struct ButtonBindingState {
    key: KeyCode,
    held_count: usize,
}

impl ButtonBindingState {
    pub fn new(key: KeyCode) -> Self {
        Self { key, held_count: 0 }
    }

    pub fn is_down(&self) -> bool {
        self.held_count > 0
    }

    pub fn clear(&mut self) {
        self.held_count = 0;
    }

    fn observe(&mut self, key: KeyCode, value: i32) {
        if key != self.key {
            return;
        }
        match value {
            1 => self.held_count = self.held_count.saturating_add(1),
            0 => self.held_count = self.held_count.saturating_sub(1),
            _ => {}
        }
    }
}

pub fn process_event(
    pending_batch: &mut Vec<InputEvent>,
    relay_enabled: bool,
    event: InputEvent,
    trigger_binding: Option<&mut ButtonBindingState>,
    virtual_mouse: &mut VirtualMouse,
) -> Result<()> {
    match event.destructure() {
        EventSummary::Synchronization(_, SynchronizationCode::SYN_REPORT, _) => {
            if relay_enabled {
                virtual_mouse.emit_batch(pending_batch)?;
            }
            pending_batch.clear();
        }
        EventSummary::RelativeAxis(_, axis, _) if is_supported_relative_axis(axis) => {
            if relay_enabled {
                pending_batch.push(event);
            }
        }
        EventSummary::Key(_, key, value) if is_supported_button(key) => {
            if let Some(trigger_binding) = trigger_binding {
                trigger_binding.observe(key, value);
            }
            if relay_enabled && matches!(value, 0 | 1) {
                pending_batch.push(event);
            }
        }
        _ => {}
    }

    Ok(())
}

fn is_supported_relative_axis(axis: RelativeAxisCode) -> bool {
    matches!(
        axis,
        RelativeAxisCode::REL_X
            | RelativeAxisCode::REL_Y
            | RelativeAxisCode::REL_WHEEL
            | RelativeAxisCode::REL_HWHEEL
    )
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
