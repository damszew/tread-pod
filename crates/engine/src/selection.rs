//! Treadmills seen advertising FTMS, and the choice among them.
//!
//! The backend scans continuously and reports every FTMS advertisement. Entries keep their
//! discovery order, so cycling with the button visits every treadmill in a stable order even
//! though RSSI fluctuates.

use heapless::{String, Vec};

use crate::config::Address;

pub const MAX_CANDIDATES: usize = 16;

/// A treadmill not heard from for this long is dropped.
pub const CANDIDATE_TTL_MS: u64 = 10_000;

pub type Name = String<32>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub address: Address,
    pub name: Name,
    /// Smoothed RSSI, dBm.
    pub rssi: i8,
    pub last_seen: u64,
}

#[derive(Debug, Default)]
pub struct Candidates {
    list: Vec<Candidate, MAX_CANDIDATES>,
}

impl Candidates {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an advertisement. `name` may be `None` when it came without one (names often
    /// arrive in a separate scan response); the known name is then kept.
    pub fn seen(&mut self, now_ms: u64, address: Address, rssi: i8, name: Option<&str>) {
        let index = if let Some(i) = self.position(address) {
            let c = &mut self.list[i];
            // Exponential smoothing, 1/4 weight for the new reading.
            let smoothed = (3 * i16::from(c.rssi) + i16::from(rssi)) / 4;
            c.rssi = i8::try_from(smoothed).unwrap_or(rssi);
            i
        } else {
            if self.list.is_full() {
                self.evict_oldest();
            }
            let candidate = Candidate {
                address,
                name: Name::new(),
                rssi,
                last_seen: now_ms,
            };
            // Not full after evicting.
            let _ = self.list.push(candidate);
            self.list.len() - 1
        };
        let c = &mut self.list[index];
        c.last_seen = now_ms;
        if let Some(name) = name {
            c.name = truncate(name);
        }
    }

    /// Marks a treadmill as present without an advertisement: a connected treadmill usually stops
    /// advertising, but is still there.
    pub fn touch(&mut self, now_ms: u64, address: Address) {
        if let Some(i) = self.position(address) {
            self.list[i].last_seen = now_ms;
        }
    }

    /// Drops treadmills not seen for [`CANDIDATE_TTL_MS`].
    pub fn expire(&mut self, now_ms: u64) {
        self.list
            .retain(|c| c.last_seen + CANDIDATE_TTL_MS >= now_ms);
    }

    pub fn get(&self, address: Address) -> Option<&Candidate> {
        self.list.iter().find(|c| c.address == address)
    }

    /// In discovery order.
    pub fn iter(&self) -> impl Iterator<Item = &Candidate> {
        self.list.iter()
    }

    /// The strongest signal; the earliest discovered wins a tie.
    pub fn strongest(&self) -> Option<Address> {
        self.list
            .iter()
            .rev()
            .max_by_key(|c| c.rssi)
            .map(|c| c.address)
    }

    /// The candidate after `current` in discovery order, wrapping around; the first one if
    /// `current` is `None` or gone. `None` if there is nothing else to switch to.
    pub fn after(&self, current: Option<Address>) -> Option<Address> {
        let next = match current.and_then(|a| self.position(a)) {
            Some(i) => self.list.get(i + 1).or_else(|| self.list.first()),
            None => self.list.first(),
        };
        next.map(|c| c.address).filter(|&a| Some(a) != current)
    }

    fn position(&self, address: Address) -> Option<usize> {
        self.list.iter().position(|c| c.address == address)
    }

    fn evict_oldest(&mut self) {
        if let Some(i) = (0..self.list.len()).min_by_key(|&i| self.list[i].last_seen) {
            self.list.remove(i);
        }
    }
}

/// Cuts `name` to fit [`Name`], on a character boundary.
fn truncate(name: &str) -> Name {
    let mut out = Name::new();
    for ch in name.chars() {
        if out.push(ch).is_err() {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Address = [0xa; 6];
    const B: Address = [0xb; 6];
    const C: Address = [0xc; 6];

    #[test]
    fn keeps_name_from_scan_response() {
        let mut c = Candidates::new();
        c.seen(0, A, -60, Some("FS-4C02A8"));
        c.seen(100, A, -60, None);
        assert_eq!(c.get(A).unwrap().name, "FS-4C02A8");
    }

    #[test]
    fn long_names_are_cut_on_a_char_boundary() {
        let mut c = Candidates::new();
        let name = "ż".repeat(20); // 40 bytes
        c.seen(0, A, -60, Some(&name));
        assert_eq!(c.get(A).unwrap().name.as_str(), "ż".repeat(16));
    }

    #[test]
    fn rssi_is_smoothed() {
        let mut c = Candidates::new();
        c.seen(0, A, -60, None);
        c.seen(100, A, -40, None);
        assert_eq!(c.get(A).unwrap().rssi, -55);
    }

    #[test]
    fn strongest_wins() {
        let mut c = Candidates::new();
        c.seen(0, A, -70, None);
        c.seen(0, B, -50, None);
        c.seen(0, C, -50, None);
        assert_eq!(c.strongest(), Some(B));
    }

    #[test]
    fn after_cycles_in_discovery_order() {
        let mut c = Candidates::new();
        assert_eq!(c.after(None), None);
        c.seen(0, A, -70, None);
        assert_eq!(c.after(Some(A)), None, "nothing else to switch to");
        assert_eq!(c.after(None), Some(A));
        c.seen(0, B, -50, None);
        c.seen(0, C, -60, None);
        assert_eq!(c.after(Some(A)), Some(B));
        assert_eq!(c.after(Some(B)), Some(C));
        assert_eq!(c.after(Some(C)), Some(A));
        assert_eq!(c.after(Some([0; 6])), Some(A), "gone: start over");
    }

    #[test]
    fn expires_and_touch_keeps_alive() {
        let mut c = Candidates::new();
        c.seen(0, A, -70, None);
        c.seen(0, B, -70, None);
        c.touch(8_000, A);
        c.expire(12_000);
        assert!(c.get(A).is_some());
        assert!(c.get(B).is_none());
    }

    #[test]
    fn full_list_evicts_least_recently_seen() {
        let mut c = Candidates::new();
        for i in 0..MAX_CANDIDATES {
            let i = u8::try_from(i).unwrap();
            c.seen(u64::from(i) + 10, [i; 6], -70, None);
        }
        c.seen(5, [0; 6], -70, None); // refresh keeps it oldest
        c.seen(100, [0xff; 6], -70, None);
        assert!(c.get([0; 6]).is_none());
        assert!(c.get([0xff; 6]).is_some());
    }
}
