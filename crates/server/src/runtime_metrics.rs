use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use protocol::common::Usage;
use serde::Serialize;

const MAX_BUCKETS: usize = 24 * 60 + 8;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TokenBucketSnapshot {
    pub minute_start_unix: i64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub completed_requests: u64,
    pub usage_requests: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TokenSeriesSnapshot {
    pub range_minutes: u32,
    pub partial_usage: bool,
    pub buckets: Vec<TokenBucketSnapshot>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeMetricsSnapshot {
    pub uptime_seconds: u64,
    pub request_count: u64,
    pub completed_request_count: u64,
    pub error_count: u64,
    pub store_size: usize,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct TokenBucket {
    minute_start_unix: i64,
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    completed_requests: u64,
    usage_requests: u64,
}

#[derive(Debug, Clone)]
struct RuntimeMetricsState {
    started_at_unix: i64,
    request_count: u64,
    completed_request_count: u64,
    error_count: u64,
    store_size: usize,
    last_error: Option<String>,
    buckets: VecDeque<TokenBucket>,
}

impl Default for RuntimeMetricsState {
    fn default() -> Self {
        Self {
            started_at_unix: current_unix_seconds(),
            request_count: 0,
            completed_request_count: 0,
            error_count: 0,
            store_size: 0,
            last_error: None,
            buckets: VecDeque::new(),
        }
    }
}

#[derive(Clone, Default)]
pub struct RuntimeMetrics {
    inner: Arc<Mutex<RuntimeMetricsState>>,
}

impl RuntimeMetrics {
    pub fn record_request_received(&self) {
        let mut state = self.inner.lock().expect("runtime metrics lock poisoned");
        state.request_count += 1;
    }

    pub fn record_request_error(&self, message: impl Into<String>) {
        let mut state = self.inner.lock().expect("runtime metrics lock poisoned");
        state.error_count += 1;
        state.last_error = Some(message.into());
    }

    pub fn record_request_completed(&self, usage: Option<&Usage>) {
        self.record_request_completed_at(usage, current_unix_seconds());
    }

    pub fn record_request_completed_at(&self, usage: Option<&Usage>, unix_seconds: i64) {
        let mut state = self.inner.lock().expect("runtime metrics lock poisoned");
        state.completed_request_count += 1;
        let bucket = ensure_bucket(&mut state.buckets, unix_seconds);
        bucket.completed_requests += 1;
        if let Some(usage) = usage {
            bucket.input_tokens += usage.input_tokens as u64;
            bucket.output_tokens += usage.output_tokens as u64;
            bucket.total_tokens += usage.total_tokens as u64;
            bucket.usage_requests += 1;
        }
        trim_old_buckets(&mut state.buckets);
    }

    pub fn set_store_size(&self, size: usize) {
        let mut state = self.inner.lock().expect("runtime metrics lock poisoned");
        state.store_size = size;
    }

    pub fn snapshot(&self) -> RuntimeMetricsSnapshot {
        let state = self.inner.lock().expect("runtime metrics lock poisoned");
        let uptime_seconds = current_unix_seconds()
            .saturating_sub(state.started_at_unix)
            .max(0) as u64;
        RuntimeMetricsSnapshot {
            uptime_seconds,
            request_count: state.request_count,
            completed_request_count: state.completed_request_count,
            error_count: state.error_count,
            store_size: state.store_size,
            last_error: state.last_error.clone(),
        }
    }

    pub fn token_series(&self, range_minutes: u32) -> TokenSeriesSnapshot {
        self.token_series_at(range_minutes, current_unix_seconds())
    }

    fn token_series_at(&self, range_minutes: u32, now_unix_seconds: i64) -> TokenSeriesSnapshot {
        let mut state = self.inner.lock().expect("runtime metrics lock poisoned");
        trim_old_buckets(&mut state.buckets);

        let now_minute = floor_to_minute(now_unix_seconds);
        let first_minute = now_minute - ((range_minutes as i64).saturating_sub(1) * 60);
        let mut cursor = first_minute;
        let mut idx = 0usize;
        let mut buckets = Vec::with_capacity(range_minutes as usize);
        let mut partial_usage = false;

        while cursor <= now_minute {
            while idx < state.buckets.len() && state.buckets[idx].minute_start_unix < cursor {
                idx += 1;
            }

            let bucket =
                if idx < state.buckets.len() && state.buckets[idx].minute_start_unix == cursor {
                    let bucket = &state.buckets[idx];
                    if bucket.completed_requests > bucket.usage_requests {
                        partial_usage = true;
                    }
                    TokenBucketSnapshot {
                        minute_start_unix: bucket.minute_start_unix,
                        input_tokens: bucket.input_tokens,
                        output_tokens: bucket.output_tokens,
                        total_tokens: bucket.total_tokens,
                        completed_requests: bucket.completed_requests,
                        usage_requests: bucket.usage_requests,
                    }
                } else {
                    TokenBucketSnapshot {
                        minute_start_unix: cursor,
                        input_tokens: 0,
                        output_tokens: 0,
                        total_tokens: 0,
                        completed_requests: 0,
                        usage_requests: 0,
                    }
                };

            buckets.push(bucket);
            cursor += 60;
        }

        TokenSeriesSnapshot {
            range_minutes,
            partial_usage,
            buckets,
        }
    }
}

fn ensure_bucket(buckets: &mut VecDeque<TokenBucket>, unix_seconds: i64) -> &mut TokenBucket {
    let minute_start_unix = floor_to_minute(unix_seconds);
    if buckets
        .back()
        .is_none_or(|bucket| bucket.minute_start_unix != minute_start_unix)
    {
        buckets.push_back(TokenBucket {
            minute_start_unix,
            ..Default::default()
        });
    }
    buckets.back_mut().expect("bucket just pushed")
}

fn trim_old_buckets(buckets: &mut VecDeque<TokenBucket>) {
    while buckets.len() > MAX_BUCKETS {
        buckets.pop_front();
    }
}

fn floor_to_minute(unix_seconds: i64) -> i64 {
    unix_seconds - (unix_seconds.rem_euclid(60))
}

fn current_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u32, output: u32) -> Usage {
        Usage {
            input_tokens: input,
            output_tokens: output,
            total_tokens: input + output,
            input_tokens_details: None,
            output_tokens_details: None,
        }
    }

    #[test]
    fn token_series_groups_usage_by_minute_and_marks_partial_usage() {
        let metrics = RuntimeMetrics::default();
        let base = 1_700_000_000;
        metrics.record_request_completed_at(Some(&usage(10, 4)), base);
        metrics.record_request_completed_at(None, base + 4);
        metrics.record_request_completed_at(Some(&usage(3, 2)), base + 61);

        let series = metrics.token_series_at(3, base + 61);
        assert_eq!(series.buckets.len(), 3);
        assert!(series.partial_usage);
        let populated: Vec<_> = series
            .buckets
            .into_iter()
            .filter(|bucket| bucket.completed_requests > 0)
            .collect();
        assert_eq!(populated.len(), 2);
        assert_eq!(populated[0].total_tokens, 14);
        assert_eq!(populated[0].completed_requests, 2);
        assert_eq!(populated[0].usage_requests, 1);
        assert_eq!(populated[1].total_tokens, 5);
    }

    #[test]
    fn snapshot_tracks_counts_and_store_size() {
        let metrics = RuntimeMetrics::default();
        metrics.record_request_received();
        metrics.record_request_received();
        metrics.record_request_error("boom");
        metrics.record_request_completed(Some(&usage(2, 3)));
        metrics.set_store_size(7);

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.request_count, 2);
        assert_eq!(snapshot.completed_request_count, 1);
        assert_eq!(snapshot.error_count, 1);
        assert_eq!(snapshot.store_size, 7);
        assert_eq!(snapshot.last_error.as_deref(), Some("boom"));
    }
}
