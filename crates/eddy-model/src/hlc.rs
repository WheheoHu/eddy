use std::fmt;
use std::str::FromStr;

use parking_lot::Mutex;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Hybrid logical clock timestamp. The derived ordering (wall time, counter,
/// node) is the total order used for last-writer-wins merges, and the string
/// form sorts the same way.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hlc {
    pub wall_ms: u64,
    pub counter: u32,
    pub node: String,
}

impl Hlc {
    pub fn zero() -> Self {
        Hlc {
            wall_ms: 0,
            counter: 0,
            node: String::new(),
        }
    }
}

impl fmt::Display for Hlc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:013}-{:06}-{}", self.wall_ms, self.counter, self.node)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid HLC timestamp")]
pub struct ParseHlcError;

impl FromStr for Hlc {
    type Err = ParseHlcError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.splitn(3, '-');
        let wall_ms = parts
            .next()
            .and_then(|p| p.parse().ok())
            .ok_or(ParseHlcError)?;
        let counter = parts
            .next()
            .and_then(|p| p.parse().ok())
            .ok_or(ParseHlcError)?;
        let node = parts.next().ok_or(ParseHlcError)?.to_string();
        Ok(Hlc {
            wall_ms,
            counter,
            node,
        })
    }
}

impl Serialize for Hlc {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Hlc {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Issues monotonically increasing timestamps for one node and folds in
/// timestamps observed from other nodes.
pub struct HlcClock {
    node: String,
    last: Mutex<(u64, u32)>,
}

impl HlcClock {
    pub fn new(node: impl Into<String>) -> Self {
        HlcClock {
            node: node.into(),
            last: Mutex::new((0, 0)),
        }
    }

    pub fn node(&self) -> &str {
        &self.node
    }

    pub fn now(&self) -> Hlc {
        self.tick(physical_ms())
    }

    fn tick(&self, physical: u64) -> Hlc {
        let mut last = self.last.lock();
        let (wall, counter) = if physical > last.0 {
            (physical, 0)
        } else {
            (last.0, last.1 + 1)
        };
        *last = (wall, counter);
        Hlc {
            wall_ms: wall,
            counter,
            node: self.node.clone(),
        }
    }

    pub fn observe(&self, remote: &Hlc) {
        let mut last = self.last.lock();
        if (remote.wall_ms, remote.counter) > *last {
            *last = (remote.wall_ms, remote.counter);
        }
    }
}

fn physical_ms() -> u64 {
    crate::now_ms().max(0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_is_monotonic_even_when_wall_time_stalls() {
        let clock = HlcClock::new("a");
        let first = clock.tick(1000);
        let second = clock.tick(1000);
        let third = clock.tick(999);
        assert!(first < second && second < third);
        assert_eq!(third.wall_ms, 1000);
    }

    #[test]
    fn observing_a_remote_timestamp_moves_the_clock_forward() {
        let clock = HlcClock::new("a");
        clock.observe(&Hlc {
            wall_ms: 5_000,
            counter: 7,
            node: "b".into(),
        });
        let next = clock.tick(10);
        assert!(
            next > Hlc {
                wall_ms: 5_000,
                counter: 7,
                node: "b".into()
            }
        );
    }

    #[test]
    fn string_form_round_trips_and_sorts_like_the_struct() {
        let a = Hlc {
            wall_ms: 99,
            counter: 3,
            node: "dev-1".into(),
        };
        let b = Hlc {
            wall_ms: 100,
            counter: 0,
            node: "dev-0".into(),
        };
        assert_eq!(a.to_string().parse::<Hlc>().unwrap(), a);
        assert!(a.to_string() < b.to_string());
        assert!(a < b);
    }
}
