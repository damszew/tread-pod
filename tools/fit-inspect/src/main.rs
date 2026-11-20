//! Prints the device and sensor records of a FIT activity: which devices the watch says took
//! part (`device_info`) and the file's creator, to see how the footpod is recorded.
//!
//! ```text
//! cargo run -p fit-inspect -- activity.fit [--all]
//! ```

use std::{fs::File, path::PathBuf};

use anyhow::Context;
use clap::Parser;
use fitparser::profile::MesgNum;

#[derive(Parser)]
struct Args {
    /// The `.fit` file (Garmin Connect: activity ⚙ → Export Original, unzipped).
    file: PathBuf,
    /// Also list a count of every message type in the file.
    #[arg(long)]
    all: bool,
}

/// Messages that describe devices and sensors rather than samples.
const DEVICE_MESSAGES: &[MesgNum] = &[MesgNum::FileId, MesgNum::DeviceInfo];

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let mut file =
        File::open(&args.file).with_context(|| format!("opening {}", args.file.display()))?;
    let records = fitparser::from_reader(&mut file).context("parsing FIT file")?;

    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for record in &records {
        *counts.entry(record.kind().to_string()).or_default() += 1;
        if !DEVICE_MESSAGES.contains(&record.kind()) {
            continue;
        }
        println!("{}", record.kind());
        for field in record.fields() {
            let units = if field.units().is_empty() {
                String::new()
            } else {
                format!(" {}", field.units())
            };
            println!("  {} = {}{units}", field.name(), field.value());
        }
    }

    if args.all {
        println!("message counts");
        for (kind, count) in counts {
            println!("  {kind}: {count}");
        }
    }
    Ok(())
}
