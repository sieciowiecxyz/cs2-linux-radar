#[derive(Debug, Clone, Copy)]
pub struct KillSwitch {
    enabled: bool,
}

impl KillSwitch {
    pub fn new_enabled() -> Self {
        Self { enabled: true }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn toggle(&mut self) -> bool {
        self.enabled = !self.enabled;
        self.enabled
    }

    pub fn disable(&mut self) {
        self.enabled = false;
    }
}
