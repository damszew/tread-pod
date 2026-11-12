//! Spike: capture a treadmill's raw FTMS traffic so `engine` can be tested against real packets.
//!
//! Scans for devices advertising the Fitness Machine service (0x1826), connects to the chosen one,
//! reads its static characteristics and logs every notification until Ctrl-C (or `--duration`).
//!
//! Output format (one capture per file, see `crates/engine/tests/fixtures/ftms/README.md`):
//!
//! ```text
//! # comment / metadata lines
//! <ms since subscribe> <16-bit characteristic UUID, hex> <payload hex>
//! ```

use std::{
    collections::HashMap,
    fmt::Write as _,
    io::Write as _,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, bail};
use bluer::{Adapter, AdapterEvent, Address, Device, Uuid, gatt::remote::Characteristic};
use clap::Parser;
use futures::{StreamExt, stream::SelectAll};

const FTMS_SERVICE: u16 = 0x1826;

/// Characteristics we capture: (UUID, name). Reads happen once; notifies are logged continuously.
const CHARACTERISTICS: &[(u16, &str)] = &[
    (0x2acc, "fitness-machine-feature"),
    (0x2acd, "treadmill-data"),
    (0x2ad3, "training-status"),
    (0x2ad4, "supported-speed-range"),
    (0x2ada, "fitness-machine-status"),
];

#[derive(Parser)]
#[command(about)]
struct Args {
    /// Treadmill to capture. Without it, the one with the strongest RSSI is used.
    #[arg(long)]
    address: Option<Address>,

    /// Only scan and list FTMS devices with their RSSI, don't connect.
    #[arg(long)]
    list: bool,

    /// How long to scan before picking a device.
    #[arg(long, default_value = "5", value_name = "SECONDS")]
    scan: u64,

    /// Stop capturing after this long (default: until Ctrl-C).
    #[arg(long, value_name = "SECONDS")]
    duration: Option<u64>,

    /// Directory for the capture file.
    #[arg(long, default_value = "crates/engine/tests/fixtures/ftms")]
    out_dir: PathBuf,

    /// Free-form note stored in the capture header (e.g. "warm-up, 6 to 12 km/h").
    #[arg(long, default_value = "")]
    note: String,
}

struct Seen {
    name: String,
    rssi: Option<i16>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let session = bluer::Session::new().await?;
    let adapter = session.default_adapter().await?;
    adapter.set_powered(true).await?;

    let seen = scan(&adapter, Duration::from_secs(args.scan)).await?;
    let mut by_rssi: Vec<_> = seen.iter().collect();
    by_rssi.sort_by_key(|(_, s)| std::cmp::Reverse(s.rssi.unwrap_or(i16::MIN)));
    println!("FTMS devices:");
    for (address, s) in &by_rssi {
        let rssi = s.rssi.map_or_else(|| "?".into(), |r| r.to_string());
        println!("  {address}  {rssi:>4} dBm  {}", s.name);
    }
    if args.list {
        return Ok(());
    }

    let address = match args.address {
        Some(address) => address,
        None => match by_rssi.first() {
            Some((address, _)) => **address,
            None => bail!("no FTMS devices found"),
        },
    };
    let info = seen.get(&address);
    let device = adapter.device(address)?;
    println!("connecting to {address}…");
    device.connect().await.context("connect")?;

    let result = capture(&args, &device, address, info).await;
    let _ = device.disconnect().await;
    result
}

/// Scans for `duration` and returns devices advertising the FTMS service.
async fn scan(adapter: &Adapter, duration: Duration) -> anyhow::Result<HashMap<Address, Seen>> {
    println!("scanning for {}s…", duration.as_secs());
    let mut events = adapter.discover_devices().await?;
    let mut seen = HashMap::new();
    let deadline = tokio::time::sleep(duration);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            () = &mut deadline => break,
            Some(AdapterEvent::DeviceAdded(address)) = events.next() => {
                let device = adapter.device(address)?;
                // FTMS devices put the service UUID in a Service Data AD (with machine-type
                // flags); not all of them also list it among the advertised UUIDs.
                let ftms = uuid16(FTMS_SERVICE);
                let advertises_ftms = device.uuids().await?.is_some_and(|u| u.contains(&ftms))
                    || device.service_data().await?.is_some_and(|d| d.contains_key(&ftms));
                if advertises_ftms {
                    let name = device.name().await?.unwrap_or_default();
                    let rssi = device.rssi().await?;
                    seen.insert(address, Seen { name, rssi });
                }
            }
        }
    }
    // Refresh RSSI for everything we kept: the first value can be stale.
    for (address, s) in &mut seen {
        if let Ok(Some(rssi)) = adapter.device(*address)?.rssi().await {
            s.rssi = Some(rssi);
        }
    }
    Ok(seen)
}

async fn capture(
    args: &Args,
    device: &Device,
    address: Address,
    info: Option<&Seen>,
) -> anyhow::Result<()> {
    let characteristics = ftms_characteristics(device).await?;

    std::fs::create_dir_all(&args.out_dir)?;
    let name = info.map_or("unknown", |s| s.name.as_str());
    let unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let path = args.out_dir.join(format!("{}-{unix}.txt", slug(name)));
    let mut out = std::fs::File::create(&path)?;

    writeln!(out, "# ftms-sniff capture")?;
    writeln!(out, "# device: {name} ({address})")?;
    if let Some(rssi) = info.and_then(|s| s.rssi) {
        writeln!(out, "# rssi: {rssi} dBm")?;
    }
    writeln!(out, "# unix-time: {unix}")?;
    if !args.note.is_empty() {
        writeln!(out, "# note: {}", args.note)?;
    }
    for (uuid, name) in CHARACTERISTICS {
        let found = characteristics.iter().any(|(u, _)| u == uuid);
        writeln!(
            out,
            "# {uuid:04x} {name}: {}",
            if found { "present" } else { "absent" }
        )?;
    }

    // Static characteristics first, logged at t=0.
    for (uuid, ch) in &characteristics {
        if ch.flags().await?.read {
            match ch.read().await {
                Ok(value) => {
                    writeln!(out, "0 {uuid:04x} {}", hex(&value))?;
                    println!("read {uuid:04x}: {}", hex(&value));
                }
                Err(err) => println!("read {uuid:04x} failed: {err}"),
            }
        }
    }

    let mut notifications = SelectAll::new();
    for (uuid, ch) in &characteristics {
        let flags = ch.flags().await?;
        if flags.notify || flags.indicate {
            let uuid = *uuid;
            notifications.push(ch.notify().await?.map(move |value| (uuid, value)).boxed());
        }
    }
    if notifications.is_empty() {
        bail!("treadmill exposes no notifiable FTMS characteristics");
    }

    println!("capturing to {} (Ctrl-C to stop)", path.display());
    let start = Instant::now();
    let stop = async {
        match args.duration {
            Some(secs) => tokio::time::sleep(Duration::from_secs(secs)).await,
            None => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    };
    tokio::pin!(stop);
    let mut count = 0usize;
    loop {
        tokio::select! {
            () = &mut stop => break,
            next = notifications.next() => {
                let Some((uuid, value)) = next else { bail!("treadmill disconnected") };
                let ms = start.elapsed().as_millis();
                writeln!(out, "{ms} {uuid:04x} {}", hex(&value))?;
                out.flush()?;
                count += 1;
                println!("{ms:>8} {uuid:04x} {}{}", hex(&value), summary(uuid, &value));
            }
        }
    }
    println!("saved {count} notifications to {}", path.display());
    Ok(())
}

/// FTMS characteristics we care about, as (16-bit UUID, handle).
async fn ftms_characteristics(device: &Device) -> anyhow::Result<Vec<(u16, Characteristic)>> {
    let mut found = Vec::new();
    for service in device.services().await? {
        if service.uuid().await? != uuid16(FTMS_SERVICE) {
            continue;
        }
        for ch in service.characteristics().await? {
            let uuid = ch.uuid().await?;
            if let Some(&(short, _)) = CHARACTERISTICS.iter().find(|(u, _)| uuid16(*u) == uuid) {
                found.push((short, ch));
            }
        }
    }
    if found.is_empty() {
        bail!("device has no FTMS service");
    }
    Ok(found)
}

/// Just enough decoding to sanity-check a capture live. The real parser lives in `engine`.
fn summary(uuid: u16, value: &[u8]) -> String {
    if uuid != 0x2acd || value.len() < 2 {
        return String::new();
    }
    let flags = u16::from_le_bytes([value[0], value[1]]);
    let more_data = flags & 1 != 0;
    let mut s = format!("  flags={flags:#06x}");
    if more_data {
        s.push_str(" (more data, no speed)");
    } else if let Some(speed) = value.get(2..4) {
        let speed = u16::from_le_bytes([speed[0], speed[1]]);
        let _ = write!(s, " speed={}.{:02} km/h", speed / 100, speed % 100);
    }
    s
}

const fn uuid16(short: u16) -> Uuid {
    Uuid::from_u128(0x0000_0000_0000_1000_8000_0080_5f9b_34fb | ((short as u128) << 96))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn slug(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "unknown".into()
    } else {
        slug.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid16_expands_to_bluetooth_base() {
        assert_eq!(
            uuid16(0x2acd).to_string(),
            "00002acd-0000-1000-8000-00805f9b34fb"
        );
    }

    #[test]
    fn summary_reads_speed_unless_more_data() {
        // flags 0x0000, speed 0x041a = 1050 -> 10.50 km/h
        assert!(summary(0x2acd, &[0x00, 0x00, 0x1a, 0x04]).contains("speed=10.50 km/h"));
        assert!(summary(0x2acd, &[0x01, 0x00, 0x1a, 0x04]).contains("more data"));
    }
}
