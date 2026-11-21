//! Prints a tread-pod flash log: the raw `evlog` partition, as `fw-log` reads it off the device.
//!
//! Usage: `event-log <dump>`

use std::{env, fs};

use anyhow::{Context, bail};
use engine::event_log::{Entry, EventLog, MemStorage};

fn main() -> anyhow::Result<()> {
    let mut args = env::args().skip(1);
    let (Some(path), None) = (args.next(), args.next()) else {
        bail!("usage: event-log <partition dump>");
    };
    let mut bytes = fs::read(&path).with_context(|| format!("reading {path}"))?;

    // RAM storage can't fail.
    let Ok(mut log) = EventLog::open(MemStorage(&mut bytes));
    let mut last_uptime = None;
    let Ok(()) = log.for_each(|entry| {
        println!("{}", line(&entry, last_uptime));
        last_uptime = Some(entry.uptime_ms);
    });
    if last_uptime.is_none() {
        println!("The log is empty.");
    }
    Ok(())
}

/// One entry, after a blank line when the uptime went back: the board restarted.
fn line(entry: &Entry, last_uptime: Option<u64>) -> String {
    let boot = if last_uptime.is_some_and(|t| entry.uptime_ms < t) {
        "\n"
    } else {
        ""
    };
    format!(
        "{boot}#{:<6} {:>12}  {}",
        entry.seq,
        uptime(entry.uptime_ms),
        entry.text
    )
}

/// `h:mm:ss.mmm` since boot.
fn uptime(ms: u64) -> String {
    let (s, ms) = (ms / 1000, ms % 1000);
    format!("{}:{:02}:{:02}.{ms:03}", s / 3600, s / 60 % 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restart_gets_a_blank_line() {
        let entry = Entry {
            seq: 42,
            uptime_ms: 1_682_250,
            text: "W session: treadmill data gap: 3500 ms",
        };
        let expected = "#42      0:28:02.250  W session: treadmill data gap: 3500 ms";
        assert_eq!(line(&entry, Some(1_000)), expected);
        assert_eq!(line(&entry, Some(2_000_000)), format!("\n{expected}"));
    }
}
