//! Where each device is in the change feed, kept in memory: the source of
//! the sync lines in the log and of the sync position on the admin page.
//! Lost on restart and filled in again as devices sync.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// A long catch-up reports how far it is this often.
const REPORT_EVERY_OPS: u64 = 50_000;
const REPORT_EVERY: Duration = Duration::from_secs(10);

/// A device's place in the change feed after its latest pull.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    /// The last change it was sent.
    pub cursor: i64,
    /// When it pulled (Unix ms).
    pub at: i64,
}

struct CatchUp {
    started: Instant,
    sent: u64,
    reported: Instant,
    reported_sent: u64,
}

struct Entry {
    position: Position,
    catch_up: Option<CatchUp>,
}

#[derive(Default)]
pub struct SyncLog {
    devices: Mutex<HashMap<String, Entry>>,
}

/// One page handed to a device.
pub struct Pulled<'a> {
    pub device_id: &'a str,
    pub device_name: &'a str,
    pub since: i64,
    pub sent: usize,
    pub cursor: i64,
    pub more: bool,
    pub head: i64,
}

impl SyncLog {
    /// Records a pull and writes the log line it calls for, if any: a
    /// catch-up of several pages is logged at its start, now and then, and
    /// at its end; a pull that fits one page gets one line; an empty pull
    /// none.
    pub fn pulled(&self, p: Pulled) {
        let name = p.device_name;
        let mut devices = self.devices.lock();
        let entry = devices.entry(p.device_id.to_string()).or_insert(Entry {
            position: Position { cursor: 0, at: 0 },
            catch_up: None,
        });
        // A catch-up that was abandoned and starts again is a new one.
        if entry.position.cursor != p.since {
            entry.catch_up = None;
        }
        entry.position = Position {
            cursor: p.cursor,
            at: eddy_model::now_ms(),
        };
        let now = Instant::now();
        match &mut entry.catch_up {
            None if p.more => {
                tracing::info!(
                    "device {name} started syncing: {} changes behind",
                    p.head - p.since.max(0)
                );
                entry.catch_up = Some(CatchUp {
                    started: now,
                    sent: p.sent as u64,
                    reported: now,
                    reported_sent: p.sent as u64,
                });
            }
            None if p.sent > 0 => tracing::info!("device {name} pulled {} changes", p.sent),
            None => {}
            Some(run) => {
                run.sent += p.sent as u64;
                if !p.more {
                    tracing::info!(
                        "device {name} is up to date: {} changes in {:.1} s",
                        run.sent,
                        run.started.elapsed().as_secs_f64()
                    );
                    entry.catch_up = None;
                } else if run.sent - run.reported_sent >= REPORT_EVERY_OPS
                    || run.reported.elapsed() >= REPORT_EVERY
                {
                    tracing::info!(
                        "device {name} syncing: {} of {} changes",
                        run.sent,
                        run.sent + (p.head - p.cursor).max(0) as u64
                    );
                    run.reported = now;
                    run.reported_sent = run.sent;
                }
            }
        }
    }

    pub fn position(&self, device_id: &str) -> Option<Position> {
        self.devices.lock().get(device_id).map(|e| e.position)
    }

    pub fn forget(&self, device_id: &str) {
        self.devices.lock().remove(device_id);
    }
}
