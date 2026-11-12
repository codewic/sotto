//! Calls: state for each call and what a finished call leaves in the conversation.
//!
//! Signalling travels as ordinary Sotto messages (`Payload::Call`). Media goes to a call server
//! (LiveKit), end-to-end encrypted with a fresh random key that the caller puts in the offer.
//! Join tokens and keys stay in this process: views and events never carry them.

use std::collections::HashMap;
use std::time::Duration;

use serde::Serialize;
use sotto_core::{Id, hex};

/// How long an unanswered call rings.
pub const RING: Duration = Duration::from_secs(45);

/// What the call broker handed out: where to connect, and one join token per side.
pub struct RoomGrant {
    pub url: String,
    /// The room's name at the broker, for adding people later.
    pub room: String,
    pub my_token: String,
    pub their_token: String,
}

/// Everything the call window needs to join. Never serialized into events.
pub struct JoinInfo {
    pub call_id: Id,
    pub contact_id: Id,
    pub url: String,
    pub token: String,
    pub key: [u8; 32],
    pub outgoing: bool,
    pub group: bool,
}

impl std::fmt::Debug for JoinInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JoinInfo").field("call_id", &hex(&self.call_id)).field("url", &self.url).finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Outgoing,
    Incoming,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    /// Outgoing, waiting for an answer. The caller only ever sees this, never "ringing".
    Calling,
    /// Incoming, waiting for this person to answer.
    Ringing,
    /// Answered; both sides are joining the call server.
    Active,
    Ended,
}

/// Why a call ended, from this side's point of view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    /// A call that was answered and then hung up (by either side).
    Finished,
    /// Outgoing: nobody answered in time.
    NoAnswer,
    /// Outgoing: I gave up before an answer.
    Cancelled,
    /// Outgoing: they chose "can't talk now".
    TheyDeclined,
    /// Incoming: rang out or the caller gave up.
    Missed,
    /// Incoming: arrived while I was on another call.
    MissedBusy,
    /// Incoming: I declined.
    Declined,
    /// Incoming: refused because calls are off in this protection level.
    Off,
}

pub(crate) struct Call {
    pub id: Id,
    pub contact: Id,
    pub direction: Direction,
    pub phase: Phase,
    pub url: String,
    pub room: String,
    pub token: String,
    pub key: [u8; 32],
    pub started_ms: u64,
    pub answered_ms: Option<u64>,
    /// More than two people (someone was added).
    pub group: bool,
    /// Contacts this desk invited into the call, besides `contact`.
    pub invited: Vec<Id>,
}

impl Drop for Call {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.key);
        zeroize::Zeroize::zeroize(&mut self.token);
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CallView {
    pub id: String,
    pub contact_id: String,
    pub direction: Direction,
    pub phase: Phase,
    pub started_at: u64,
    pub answered_at: Option<u64>,
    pub reason: Option<EndReason>,
    pub group: bool,
    /// Contacts this desk added to the call.
    pub invited: Vec<String>,
}

impl Call {
    pub fn view(&self, reason: Option<EndReason>) -> CallView {
        CallView {
            id: hex(&self.id),
            contact_id: hex(&self.contact),
            direction: self.direction,
            phase: self.phase,
            started_at: self.started_ms,
            answered_at: self.answered_ms,
            reason,
            group: self.group,
            invited: self.invited.iter().map(|i| hex(i)).collect(),
        }
    }
}

#[derive(Default)]
pub(crate) struct Calls {
    pub map: HashMap<Id, Call>,
    /// Ring time; shortened in tests.
    pub ring: Option<Duration>,
}

impl Calls {
    pub fn ring(&self) -> Duration {
        self.ring.unwrap_or(RING)
    }

    /// Any call still going on (one at a time).
    pub fn busy(&self) -> bool {
        self.map.values().any(|c| c.phase != Phase::Ended)
    }
}

/// The line a finished call leaves in the conversation, and whether it counts as unread.
pub fn record(direction: Direction, reason: EndReason, talked_secs: Option<u64>, group: bool) -> (String, bool) {
    match reason {
        EndReason::Finished => {
            let d = talk_time(talked_secs.unwrap_or(0));
            if group {
                return (format!("Group call, {d}"), false);
            }
            match direction {
                Direction::Outgoing => (format!("Outgoing call, {d}"), false),
                Direction::Incoming => (format!("Incoming call, {d}"), false),
            }
        }
        EndReason::NoAnswer => ("No answer".into(), false),
        EndReason::Cancelled => ("Call cancelled".into(), false),
        EndReason::TheyDeclined => ("They couldn't talk".into(), false),
        EndReason::Missed => ("Missed call".into(), true),
        EndReason::MissedBusy => ("Missed call while you were on another call".into(), true),
        EndReason::Declined => ("Declined call".into(), false),
        EndReason::Off => ("Missed call (calls are off in Extreme mode)".into(), true),
    }
}

/// "35 s", "4 min 12 s", "1 h 3 min".
pub fn talk_time(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    match (h, m) {
        (0, 0) => format!("{s} s"),
        (0, m) => format!("{m} min {s} s"),
        (h, m) => format!("{h} h {m} min"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn talk_time_reads_naturally() {
        assert_eq!(talk_time(0), "0 s");
        assert_eq!(talk_time(35), "35 s");
        assert_eq!(talk_time(252), "4 min 12 s");
        assert_eq!(talk_time(3780), "1 h 3 min");
    }

    #[test]
    fn records_for_each_ending() {
        assert_eq!(record(Direction::Outgoing, EndReason::Finished, Some(252), false), ("Outgoing call, 4 min 12 s".into(), false));
        assert_eq!(record(Direction::Incoming, EndReason::Finished, Some(35), false), ("Incoming call, 35 s".into(), false));
        assert_eq!(record(Direction::Incoming, EndReason::Finished, Some(35), true), ("Group call, 35 s".into(), false));
        assert_eq!(record(Direction::Incoming, EndReason::Missed, None, false), ("Missed call".into(), true));
        assert!(!record(Direction::Outgoing, EndReason::NoAnswer, None, false).1);
    }
}
