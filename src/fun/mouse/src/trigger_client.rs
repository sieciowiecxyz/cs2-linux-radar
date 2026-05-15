use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

use fun_trigger::{read_latest_frame, socket_path_from_env};
use tracing::{info, warn};

#[derive(Clone)]
pub struct TriggerDecision {
    should_fire: Arc<AtomicBool>,
}

impl TriggerDecision {
    pub fn disabled() -> Self {
        Self {
            should_fire: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn spawn() -> Self {
        let decision = Self {
            should_fire: Arc::new(AtomicBool::new(false)),
        };
        let shared = decision.clone();
        thread::spawn(move || {
            let socket_path = socket_path_from_env();
            let period = Duration::from_millis(8);
            let mut last_warn_at: Option<Instant> = None;

            info!(socket = %socket_path.display(), "fun-mouse trigger client started");
            loop {
                match read_latest_frame(&socket_path) {
                    Ok(frame) => {
                        shared
                            .should_fire
                            .store(frame.should_fire, Ordering::Relaxed);
                        last_warn_at = None;
                        thread::sleep(period);
                    }
                    Err(err) => {
                        shared.should_fire.store(false, Ordering::Relaxed);
                        let should_log = last_warn_at
                            .is_none_or(|last| last.elapsed() >= Duration::from_secs(5));
                        if should_log {
                            warn!(error = %err, "fun-mouse trigger client read failed");
                            last_warn_at = Some(Instant::now());
                        }
                        thread::sleep(Duration::from_secs(1));
                    }
                }
            }
        });
        decision
    }

    pub fn should_fire(&self) -> bool {
        self.should_fire.load(Ordering::Relaxed)
    }
}
