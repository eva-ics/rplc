use crate::tasks::{self, ConvX};
use eva_common::EResult;
use log::warn;
use serde::{Deserialize, Deserializer};
use std::cmp::Ordering;
use std::time::Duration;
use std::time::Instant;

pub struct Loop {
    next_iter: Instant,
    interval: Duration,
    int_micros: i64,
    t: Instant,
    report: bool,
    // only Input or Program, used to mark ready
    task_kind: Option<tasks::Kind>,
    marked: bool,
}

#[negative_impl::negative_impl]
impl !Send for Loop {}

impl Loop {
    pub fn prepare0(interval: Duration) -> Self {
        Self::prepare(interval, false)
    }
    pub fn prepare_reported(interval: Duration) -> Self {
        Self::prepare(interval, true)
    }
    /// For input, program and output threads waits until the thread can pass
    /// # Panics
    ///
    /// will panic if interval in us > i64::MAX
    pub fn prepare(interval: Duration, report: bool) -> Self {
        let task_kind: Option<tasks::Kind> = if let Some(ch) = tasks::thread_name().chars().next() {
            match ch {
                'I' => {
                    tasks::wait_can_run_input();
                    Some(tasks::Kind::Input)
                }
                'P' => {
                    tasks::wait_can_run_program();
                    Some(tasks::Kind::Program)
                }
                'O' => {
                    tasks::wait_can_run_output();
                    None
                }
                _ => None,
            }
        } else {
            None
        };
        let now = Instant::now();
        Loop {
            next_iter: now + interval,
            interval,
            int_micros: i64::try_from(interval.as_micros()).unwrap(),
            t: now,
            report,
            task_kind,
            marked: task_kind.is_none(),
        }
    }

    pub fn tick(&mut self) -> bool {
        if !self.marked {
            if let Some(kind) = self.task_kind {
                tasks::mark_thread_ready(kind);
            }
            self.marked = true;
        }
        let t = Instant::now();
        let result = match t.cmp(&self.next_iter) {
            Ordering::Greater => false,
            Ordering::Equal => true,
            Ordering::Less => {
                tasks::sleep(self.next_iter - t);
                true
            }
        };
        if result {
            self.next_iter += self.interval;
        } else {
            self.next_iter = Instant::now() + self.interval;
            warn!(
                "{} loop timeout ({:?} + {:?})",
                tasks::thread_name(),
                self.interval,
                self.next_iter.elapsed()
            );
        }
        if self.report {
            let t = Instant::now();
            #[allow(clippy::cast_possible_truncation)]
            let jitter = (self.int_micros - (t.duration_since(self.t)).as_micros() as i64)
                .unsigned_abs()
                .as_u16_max();
            tasks::report_jitter(jitter);
            self.t = t;
        };
        result
    }
}

pub(crate) fn parse_interval(s: &str) -> EResult<u64> {
    if let Some(v) = s.strip_suffix("ms") {
        Ok(v.parse::<u64>()? * 1_000_000)
    } else if let Some(v) = s.strip_suffix("us") {
        Ok(v.parse::<u64>()? * 1_000)
    } else if let Some(v) = s.strip_suffix("ns") {
        Ok(v.parse::<u64>()?)
    } else if let Some(v) = s.strip_suffix('s') {
        Ok(v.parse::<u64>()? * 1_000_000_000)
    } else {
        Ok(s.parse::<u64>()? * 1_000_000_000)
    }
}

#[allow(dead_code)]
#[inline]
pub(crate) fn deserialize_interval_as_nanos<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let buf = String::deserialize(deserializer)?;
    parse_interval(&buf).map_err(serde::de::Error::custom)
}
