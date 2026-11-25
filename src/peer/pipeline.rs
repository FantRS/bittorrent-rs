use std::time::Duration;
use tokio::time::Instant;

pub const BLOCK_LEN: u32 = 1 << 14;

const MIN_DEPTH: usize = 5;
const MAX_DEPTH: usize = 250;
const QUEUE_TIME: Duration = Duration::from_secs(3);
const RATE_WINDOW: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub struct Pipeline {
    depth: usize,
    limit: usize,
    slow_start: bool,
    rate: Rate,
}

#[derive(Debug)]
struct Rate {
    bytes_per_sec: u64,
    window_bytes: u64,
    window_start: Instant,
}

impl Pipeline {
    pub fn new() -> Self {
        Self {
            depth: MIN_DEPTH,
            limit: MAX_DEPTH,
            slow_start: true,
            rate: Rate::new(),
        }
    }

    pub fn depth(&self) -> usize {
        self.depth
    }

    pub fn limit_to(&mut self, reqq: Option<usize>) {
        self.limit = reqq.unwrap_or(MAX_DEPTH).clamp(1, MAX_DEPTH);
        self.depth = self.depth.min(self.limit);
    }

    pub fn record_block(&mut self, len: usize) {
        let sampled = self.rate.record(u64::try_from(len).unwrap_or(u64::MAX));
        let target = self.target();

        if self.slow_start {
            self.depth += 1;
            if sampled && target < self.depth {
                self.slow_start = false;
            }
        }
        if !self.slow_start && sampled {
            self.depth = target;
        }
        self.depth = self.depth.clamp(MIN_DEPTH.min(self.limit), self.limit);
    }

    fn target(&self) -> usize {
        let bytes = self.rate.bytes_per_sec.saturating_mul(QUEUE_TIME.as_secs());
        usize::try_from(bytes / u64::from(BLOCK_LEN)).unwrap_or(usize::MAX)
    }
}

impl Rate {
    fn new() -> Self {
        Self {
            bytes_per_sec: 0,
            window_bytes: 0,
            window_start: Instant::now(),
        }
    }

    fn record(&mut self, bytes: u64) -> bool {
        self.window_bytes = self.window_bytes.saturating_add(bytes);
        let elapsed = self.window_start.elapsed();
        if elapsed < RATE_WINDOW {
            return false;
        }

        let millis = elapsed.as_millis().max(1);
        let sample =
            u64::try_from(u128::from(self.window_bytes) * 1000 / millis).unwrap_or(u64::MAX);
        self.bytes_per_sec = match self.bytes_per_sec {
            0 => sample,
            current => current.saturating_mul(3).saturating_add(sample) / 4,
        };
        self.window_bytes = 0;
        self.window_start = Instant::now();

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::advance;

    const BLOCK: usize = BLOCK_LEN as usize;

    #[tokio::test(start_paused = true)]
    async fn pipeline_cases() {
        let mut pipeline = Pipeline::new();
        assert_eq!(pipeline.depth(), MIN_DEPTH);

        for _ in 0..10 {
            pipeline.record_block(BLOCK);
        }
        assert_eq!(pipeline.depth(), MIN_DEPTH + 10);

        advance(Duration::from_secs(1)).await;
        pipeline.record_block(BLOCK);
        assert_eq!(pipeline.depth(), MIN_DEPTH + 11);

        for _ in 0..10 {
            advance(Duration::from_secs(1)).await;
            pipeline.record_block(BLOCK);
        }
        assert_eq!(pipeline.depth(), MIN_DEPTH);

        let mut fast = Pipeline::new();
        fast.limit_to(Some(20));
        for _ in 0..400 {
            fast.record_block(BLOCK);
        }
        advance(Duration::from_secs(1)).await;
        fast.record_block(BLOCK);
        assert_eq!(fast.depth(), 20);

        fast.limit_to(Some(0));
        assert_eq!(fast.depth(), 1);
        fast.limit_to(None);
        fast.record_block(BLOCK);
        assert!(fast.depth() >= MIN_DEPTH);
    }
}
