//! Rational monotonic deadlines; one latest opportunity per wakeup, no replay.
use std::time::Duration;
pub const WARMUP: usize = 120;
pub const MEASURED: usize = 600;
pub const TOTAL: usize = WARMUP + MEASURED;
pub const END: Duration = Duration::from_secs(12);

pub fn deadline(index: usize) -> Duration {
    Duration::from_nanos(index as u64 * 1_000_000_000 / 60)
}
pub fn segment(index: usize) -> usize {
    usize::from(index >= WARMUP)
}
#[derive(Debug, Clone, Copy)]
pub struct Opportunity {
    pub index: usize,
    pub deadline: Duration,
    pub lateness: Duration,
}
#[derive(Default)]
pub struct Scheduler {
    next: usize,
    observed: Duration,
    earliest_attempt: Duration,
    pub missed: [u64; 2],
    pub skipped: [u64; 2],
}
impl Scheduler {
    pub fn next_deadline(&self) -> Option<Duration> {
        (self.next < TOTAL).then(|| deadline(self.next).max(self.earliest_attempt))
    }
    pub fn poll(&mut self, now: Duration) -> Result<Option<Opportunity>, &'static str> {
        if now < self.observed {
            return Err("non-monotonic scheduler clock");
        }
        self.observed = now;
        if self.next == TOTAL {
            return Ok(None);
        }
        if now >= END {
            for i in self.next..TOTAL {
                self.missed[segment(i)] += 1;
            }
            self.next = TOTAL;
            return Ok(None);
        }
        if now < deadline(self.next).max(self.earliest_attempt) {
            return Ok(None);
        }
        let mut latest = self.next;
        // Fixed 720-opportunity bound, independent of overrun duration.
        while latest + 1 < TOTAL && deadline(latest + 1) <= now {
            latest += 1;
        }
        for i in self.next..latest {
            self.missed[segment(i)] += 1;
        }
        self.next = latest + 1;
        // Retain rational cadence after a late wakeup. Never issue the next
        // opportunity immediately to compensate for the previous lateness.
        self.earliest_attempt = now + (deadline(latest + 1) - deadline(latest));
        Ok(Some(Opportunity {
            index: latest,
            deadline: deadline(latest),
            lateness: now - deadline(latest),
        }))
    }
    pub fn cancel(&mut self) {
        for i in self.next..TOTAL {
            self.skipped[segment(i)] += 1;
        }
        self.next = TOTAL;
    }
}

/// Retains at most one run's 720 observations. Overflow is an explicit error.
#[derive(Debug, Default)]
pub struct Samples(Vec<Duration>);
impl Samples {
    pub fn push(&mut self, duration: Duration) -> Result<(), String> {
        if self.0.len() == TOTAL {
            return Err("bounded timing samples full".into());
        }
        self.0.push(duration);
        Ok(())
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn percentile(&self, percent: usize) -> Option<Duration> {
        if self.0.is_empty() || !(1..=100).contains(&percent) {
            return None;
        }
        let mut values = self.0.clone();
        values.sort_unstable();
        Some(values[(values.len() * percent).div_ceil(100) - 1])
    }
}
