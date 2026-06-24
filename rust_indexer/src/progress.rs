use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct ThrottledProgress {
    started_at: Instant,
    last_report_at: Instant,
}

impl ThrottledProgress {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            started_at: now,
            last_report_at: now,
        }
    }

    pub fn should_report(&mut self, item_count: u64, item_step: u64, time_step: Duration) -> bool {
        item_count == 1 || item_count % item_step == 0 || self.last_report_at.elapsed() >= time_step
    }

    pub fn mark_reported(&mut self) {
        self.last_report_at = Instant::now();
    }

    pub fn elapsed_secs(&self) -> f64 {
        self.started_at.elapsed().as_secs_f64()
    }
}
