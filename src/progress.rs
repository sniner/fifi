use std::sync::Mutex;
use std::time::{Duration, Instant};

pub trait ProgressSink: Send + Sync {
    fn tick(&self, message: &str);
}

pub struct TracingProgress {
    state: Mutex<State>,
    interval: Duration,
}

struct State {
    next_emit: Instant,
}

impl TracingProgress {
    #[must_use]
    pub fn new() -> Self {
        Self::with_timing(Duration::from_secs(2), Duration::from_secs(1))
    }

    #[must_use]
    pub fn with_timing(interval: Duration, warmup: Duration) -> Self {
        Self {
            state: Mutex::new(State {
                next_emit: Instant::now() + warmup,
            }),
            interval,
        }
    }
}

impl Default for TracingProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressSink for TracingProgress {
    fn tick(&self, message: &str) {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap();
        if now >= state.next_emit {
            tracing::info!("{message}");
            state.next_emit = now + self.interval;
        }
    }
}
