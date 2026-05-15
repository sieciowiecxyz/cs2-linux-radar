use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use evdev::{Device, EventSummary, InputEvent, KeyCode};
use tracing::{info, warn};

use crate::relay::{self, ButtonBindingState};
use crate::virtual_mouse::VirtualMouse;

pub struct PhysicalMouse {
    path: PathBuf,
    name: String,
    device: Device,
    grabbed: bool,
    pending_batch: Vec<InputEvent>,
}

impl PhysicalMouse {
    fn open(path: &Path) -> Result<Self> {
        let device = Device::open(path)
            .with_context(|| format!("open physical mouse {}", path.display()))?;
        device
            .set_nonblocking(true)
            .with_context(|| format!("set_nonblocking for {}", path.display()))?;
        let name = device.name().unwrap_or("<unnamed>").to_string();
        Ok(Self {
            path: path.to_path_buf(),
            name,
            device,
            grabbed: false,
            pending_batch: Vec::new(),
        })
    }

    fn grab(&mut self) -> Result<()> {
        if self.grabbed {
            return Ok(());
        }
        self.device
            .grab()
            .with_context(|| format!("grab {}", self.path.display()))?;
        self.grabbed = true;
        info!(path = %self.path.display(), name = %self.name, "grabbed physical mouse");
        Ok(())
    }

    fn ungrab(&mut self) {
        self.pending_batch.clear();
        if !self.grabbed {
            return;
        }
        if let Err(err) = self.device.ungrab() {
            warn!(
                path = %self.path.display(),
                name = %self.name,
                error = %err,
                "failed to ungrab physical mouse"
            );
            return;
        }
        self.grabbed = false;
        info!(path = %self.path.display(), name = %self.name, "ungrabbed physical mouse");
    }
}

pub struct PhysicalMouseSet {
    mice: Vec<PhysicalMouse>,
}

impl PhysicalMouseSet {
    pub fn open(paths: &[PathBuf]) -> Result<Self> {
        if paths.is_empty() {
            bail!("physical mouse path list must not be empty");
        }

        let mice = paths
            .iter()
            .map(|path| PhysicalMouse::open(path))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { mice })
    }

    pub fn grab_all(&mut self) -> Result<()> {
        for idx in 0..self.mice.len() {
            if let Err(err) = self.mice[idx].grab() {
                for grabbed in &mut self.mice[..idx] {
                    grabbed.ungrab();
                }
                return Err(err);
            }
        }
        Ok(())
    }

    pub fn ungrab_all(&mut self) {
        for mouse in &mut self.mice {
            mouse.ungrab();
        }
    }

    pub fn pump(
        &mut self,
        relay_enabled: bool,
        trigger_binding: Option<&mut ButtonBindingState>,
        virtual_mouse: &mut VirtualMouse,
    ) -> Result<bool> {
        let mut saw_events = false;
        let mut trigger_binding = trigger_binding;

        for mouse in &mut self.mice {
            match mouse.device.fetch_events() {
                Ok(events) => {
                    for event in events {
                        saw_events = true;
                        relay::process_event(
                            &mut mouse.pending_batch,
                            relay_enabled,
                            event,
                            trigger_binding.as_deref_mut(),
                            virtual_mouse,
                        )?;
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
                Err(err) => {
                    return Err(err)
                        .with_context(|| format!("fetch_events for {}", mouse.path.display()));
                }
            }
        }

        Ok(saw_events)
    }

    pub fn poll_first_pressed_button(&mut self) -> Result<Option<KeyCode>> {
        for mouse in &mut self.mice {
            match mouse.device.fetch_events() {
                Ok(events) => {
                    for event in events {
                        match event.destructure() {
                            EventSummary::Key(_, key, 1) => return Ok(Some(key)),
                            EventSummary::Synchronization(_, _, _) => {
                                mouse.pending_batch.clear();
                            }
                            _ => {}
                        }
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
                Err(err) => {
                    return Err(err)
                        .with_context(|| format!("fetch_events for {}", mouse.path.display()));
                }
            }
        }

        Ok(None)
    }
}

impl Drop for PhysicalMouseSet {
    fn drop(&mut self) {
        self.ungrab_all();
    }
}
