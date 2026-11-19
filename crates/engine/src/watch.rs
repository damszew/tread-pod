//! Which watch may connect to the footpod.
//!
//! The footpod takes one central at a time and doesn't advertise while it has one, so two watches
//! never share it. That leaves a gap: if the watch in a workout loses the link for a moment, the
//! footpod advertises again, and another paired watch searching nearby could take it. Its owner
//! would lose speed, and the other watch would record the wrong treadmill.
//!
//! So the watch that subscribed to RSC becomes the *owner*. If its link is lost (rather than
//! closed on purpose), only the owner may connect for [`GRACE_MS`]. The backend advertises with a
//! connect allowlist while [`Gate::admission`] says so, and drops any other central that gets in.

use crate::config::Address;

/// How long a watch whose link was lost keeps the footpod to itself.
pub const GRACE_MS: u64 = 120_000;

/// Why a central left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leave {
    /// It closed the link on purpose (the workout ended or was discarded), or the footpod did.
    Deliberate,
    /// Anything else: out of range, supervision timeout, radio trouble.
    Lost,
}

impl Leave {
    /// Classifies an HCI disconnect reason (Core spec, Vol 1, Part F).
    pub fn from_hci(reason: u8) -> Self {
        match reason {
            // Remote User Terminated Connection, Connection Terminated By Local Host.
            0x13 | 0x16 => Self::Deliberate,
            _ => Self::Lost,
        }
    }
}

/// Who may connect now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Anyone,
    /// Only this watch, until its grace period ends.
    Only(Address),
}

#[derive(Debug, Default)]
pub struct Gate {
    /// The watch that last subscribed to RSC, until it leaves on purpose or its grace runs out.
    owner: Option<Address>,
    /// Set while the owner is away after a lost link.
    grace_until: Option<u64>,
}

impl Gate {
    pub const fn new() -> Self {
        Self {
            owner: None,
            grace_until: None,
        }
    }

    /// A central connected. Returns `false` if it must be dropped: someone else's grace period
    /// is running (the allowlist should already have kept it out).
    pub fn connected(&mut self, now_ms: u64, address: Address) -> bool {
        match self.admission(now_ms) {
            Admission::Anyone => true,
            Admission::Only(owner) if owner == address => {
                self.grace_until = None;
                true
            }
            Admission::Only(_) => false,
        }
    }

    /// A central subscribed to RSC: it is the watch in this workout.
    pub fn subscribed(&mut self, address: Address) {
        self.owner = Some(address);
        self.grace_until = None;
    }

    /// A central disconnected. Only the owner's leaving matters: a phone or a rejected watch
    /// never holds the footpod.
    pub fn disconnected(&mut self, now_ms: u64, address: Address, leave: Leave) {
        if self.owner != Some(address) {
            return;
        }
        match leave {
            Leave::Deliberate => *self = Self::new(),
            Leave::Lost => self.grace_until = Some(now_ms + GRACE_MS),
        }
    }

    /// Who may connect now. Changes on its own when a grace period runs out, so the backend
    /// checks it on every tick.
    pub fn admission(&self, now_ms: u64) -> Admission {
        match (self.owner, self.grace_until) {
            (Some(owner), Some(until)) if now_ms < until => Admission::Only(owner),
            _ => Admission::Anyone,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINE: Address = [0x0c, 0x7e, 0x24, 0x2d, 0xae, 0xe6];
    const OTHER: Address = [0xaa; 6];
    const PHONE: Address = [0xbb; 6];

    /// The owner subscribed at t = 0.
    fn owned() -> Gate {
        let mut gate = Gate::new();
        assert!(gate.connected(0, MINE));
        gate.subscribed(MINE);
        gate
    }

    #[test]
    fn hci_reasons() {
        assert_eq!(Leave::from_hci(0x13), Leave::Deliberate);
        assert_eq!(Leave::from_hci(0x16), Leave::Deliberate);
        assert_eq!(Leave::from_hci(0x08), Leave::Lost);
        assert_eq!(Leave::from_hci(0x3e), Leave::Lost);
    }

    #[test]
    fn anyone_when_nobody_owns_it() {
        let mut gate = Gate::new();
        assert_eq!(gate.admission(0), Admission::Anyone);
        assert!(gate.connected(0, OTHER));
    }

    #[test]
    fn deliberate_leave_frees_it_at_once() {
        let mut gate = owned();
        gate.disconnected(1_000, MINE, Leave::Deliberate);
        assert_eq!(gate.admission(1_000), Admission::Anyone);
        assert!(gate.connected(1_000, OTHER));
    }

    #[test]
    fn lost_link_keeps_it_for_the_owner() {
        let mut gate = owned();
        gate.disconnected(1_000, MINE, Leave::Lost);
        assert_eq!(gate.admission(1_000), Admission::Only(MINE));
        assert!(!gate.connected(2_000, OTHER));
        assert!(gate.connected(3_000, MINE));
        // Back: anyone may connect again once it leaves, but it holds the only slot meanwhile.
        assert_eq!(gate.admission(3_000), Admission::Anyone);
    }

    #[test]
    fn grace_runs_out() {
        let mut gate = owned();
        gate.disconnected(1_000, MINE, Leave::Lost);
        assert_eq!(gate.admission(1_000 + GRACE_MS - 1), Admission::Only(MINE));
        assert_eq!(gate.admission(1_000 + GRACE_MS), Admission::Anyone);
        assert!(gate.connected(1_000 + GRACE_MS, OTHER));
    }

    #[test]
    fn owner_back_then_lost_again_gets_a_fresh_grace() {
        let mut gate = owned();
        gate.disconnected(1_000, MINE, Leave::Lost);
        assert!(gate.connected(100_000, MINE));
        gate.disconnected(110_000, MINE, Leave::Lost);
        assert_eq!(
            gate.admission(110_000 + GRACE_MS - 1),
            Admission::Only(MINE)
        );
    }

    #[test]
    fn rejected_watch_leaving_changes_nothing() {
        let mut gate = owned();
        gate.disconnected(1_000, MINE, Leave::Lost);
        assert!(!gate.connected(2_000, OTHER));
        gate.disconnected(2_100, OTHER, Leave::Deliberate);
        assert_eq!(gate.admission(2_100), Admission::Only(MINE));
    }

    #[test]
    fn central_that_never_subscribed_gets_no_grace() {
        let mut gate = Gate::new();
        assert!(gate.connected(0, PHONE));
        gate.disconnected(1_000, PHONE, Leave::Lost);
        assert_eq!(gate.admission(1_000), Admission::Anyone);
    }

    #[test]
    fn new_owner_replaces_the_old_one() {
        let mut gate = owned();
        gate.disconnected(1_000, MINE, Leave::Deliberate);
        assert!(gate.connected(2_000, OTHER));
        gate.subscribed(OTHER);
        gate.disconnected(3_000, OTHER, Leave::Lost);
        assert_eq!(gate.admission(3_000), Admission::Only(OTHER));
        assert!(!gate.connected(4_000, MINE));
    }
}
