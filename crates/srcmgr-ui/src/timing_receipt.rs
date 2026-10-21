//! Explicit local qualification counters. No timer, worker or per-event output.
use std::fs::File;
use std::io::Write;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy)]
pub(crate) enum Category { Keyboard, Text, Pointer, Wake, Layout, Render, Wait, Other }

const NAMES: [&str; 8] = ["keyboard", "text", "pointer", "wake", "layout", "render", "wait", "other"];
const BIN_US: u64 = 20;
const BINS: usize = 4097;

struct Group {
    calls: u64,
    samples: u64,
    max_us: u64,
    bins: Box<[u64]>,
}

impl Group {
    fn new() -> Self { Self { calls: 0, samples: 0, max_us: 0, bins: vec![0; BINS].into_boxed_slice() } }

    fn record(&mut self, elapsed: Duration) {
        let us = elapsed.as_nanos().div_ceil(1000).min(u128::from(u64::MAX)) as u64;
        self.calls += 1;
        self.max_us = self.max_us.max(us);
        if self.calls <= 5 { return; }
        self.samples += 1;
        let index = (us.saturating_sub(1) / BIN_US).min((BINS - 1) as u64) as usize;
        self.bins[index] += 1;
    }

    fn p95_upper_us(&self) -> Option<u64> {
        if self.samples < 30 { return None; }
        let rank = (self.samples * 95).div_ceil(100);
        let mut count = 0;
        for (index, number) in self.bins.iter().enumerate() {
            count += number;
            if count >= rank {
                return Some(if index == BINS - 1 { self.max_us } else { (index as u64 + 1) * BIN_US });
            }
        }
        None
    }
}

pub(crate) struct TimingReceipt {
    file: File,
    groups: [Group; 8],
    began: Instant,
    epoch_us: u128,
    activity: Box<[[u32; 5]]>,
    timeline_truncated: bool,
}

impl TimingReceipt {
    pub(crate) fn new(file: File) -> Self {
        let epoch_us = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_micros();
        Self { file, groups: std::array::from_fn(|_| Group::new()), began: Instant::now(), epoch_us, activity: vec![[0; 5]; 4096].into_boxed_slice(), timeline_truncated: false }
    }

    pub(crate) fn record(&mut self, category: Category, elapsed: Duration) {
        if matches!(category, Category::Wait) { self.groups[category as usize].calls += 1; }
        else { self.groups[category as usize].record(elapsed); }
        let end = self.began.elapsed();
        let first = end.saturating_sub(elapsed).as_secs() as usize;
        let last = end.as_secs() as usize;
        self.timeline_truncated |= last >= self.activity.len();
        let column = match category { Category::Render => 0, Category::Wake => 1, Category::Wait => 2, Category::Keyboard | Category::Text | Category::Pointer => 3, _ => 4 };
        for second in first..=last.min(self.activity.len() - 1) {
            self.activity[second][column] = self.activity[second][column].saturating_add(1);
        }
    }
}

impl Drop for TimingReceipt {
    fn drop(&mut self) {
        let mut text = String::from("{\"method\":\"Application event callback wall time and rendering measured separately; five warmups per category; 20us histogram upper bounds; maximum includes warmups; wait category is count-only; no OS queue/presentation time claim\",\"groups\":{");
        for (index, (name, group)) in NAMES.iter().zip(&self.groups).enumerate() {
            if index != 0 { text.push(','); }
            let p95 = group.p95_upper_us().map_or_else(|| "null".to_owned(), |value| value.to_string());
            text.push_str(&format!("\"{name}\":{{\"calls\":{},\"samples\":{},\"p95_upper_us\":{p95},\"max_us\":{}}}", group.calls, group.samples, group.max_us));
        }
        text.push_str(&format!("}},\"epoch_us\":{},\"timeline_truncated\":{},\"activity_columns\":[\"renders\",\"worker_wakes\",\"native_wait_callbacks\",\"input\",\"other_events\"],\"activity_seconds\":[", self.epoch_us, self.timeline_truncated));
        let seconds = (self.began.elapsed().as_secs() as usize + 1).min(self.activity.len());
        for (index, row) in self.activity.iter().take(seconds).enumerate() {
            if index != 0 { text.push(','); }
            text.push_str(&format!("[{},{},{},{},{}]", row[0], row[1], row[2], row[3], row[4]));
        }
        text.push_str("]}\n");
        if self.file.write_all(text.as_bytes()).and_then(|()| self.file.sync_all()).is_err() {
            eprintln!("SrcMgr timing receipt could not be saved; qualification evidence is unavailable");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_is_a_conservative_bound_and_cannot_hide_a_small_sample() {
        let mut group = Group::new();
        for _ in 0..5 { group.record(Duration::from_micros(9000)); }
        for value in 1..30 { group.record(Duration::from_micros(value)); }
        assert_eq!(group.p95_upper_us(), None);
        group.record(Duration::from_micros(30));
        assert_eq!(group.p95_upper_us(), Some(40));
        assert_eq!(group.max_us, 9000, "warmup stalls are still reported");
    }

    #[test]
    fn overflow_uses_the_observed_maximum_instead_of_a_false_passing_bucket() {
        let mut group = Group::new();
        for _ in 0..35 { group.record(Duration::from_millis(200)); }
        assert_eq!(group.p95_upper_us(), Some(200_000));
    }
}
