//! The logger: every line goes to USB serial as before, and info and above also to the `evlog`
//! flash partition (see `firmware/partitions.csv`), so a run can be checked after a replug. The
//! ring format lives in `engine::event_log`; this adapts the partition and does the writing.

use std::{
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
};

use engine::event_log::{EventLog, SECTOR_SIZE, Storage};
use esp_idf_svc::{
    log::{EspIdfLogFilter, EspLogger},
    partition::EspPartition,
    sys::{EspError, esp_timer_get_time},
};
use log::{Level, Log, Metadata, Record};

const PARTITION: &str = "evlog";
/// Lines waiting for the writer; more are dropped, not waited for.
const QUEUE: usize = 32;
/// Lines printed but kept off flash: our own warnings, so a failing flash can't feed itself, and
/// periodic lines that would push a run out of the ring.
pub const SERIAL_ONLY: &str = "serial";

struct Logger {
    serial: EspLogger,
    flash: SyncSender<(u64, String)>,
}

/// Installs the logger. Call first thing, before anything logs.
pub fn init() {
    let (tx, rx) = mpsc::sync_channel(QUEUE);
    let logger = Box::leak(Box::new(Logger {
        serial: EspLogger::new(EspIdfLogFilter::new()),
        flash: tx,
    }));
    logger.serial.filter().initialize();
    log::set_logger(logger).expect("the logger is set once");
    // Flash erases stall for tens of ms, so they happen here, not in whoever logs.
    if let Err(err) = thread::Builder::new()
        .name("evlog".into())
        .stack_size(6 * 1024)
        .spawn(move || write(&rx))
    {
        log::warn!(target: SERIAL_ONLY, "event log writer didn't start: {err}");
    }
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.serial.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        self.serial.log(record);
        if record.level() <= Level::Info && record.target() != SERIAL_ONLY {
            // SAFETY: reads a clock.
            let uptime_us = unsafe { esp_timer_get_time() };
            let uptime_ms = u64::try_from(uptime_us / 1000).unwrap_or(0);
            // The module alone, e.g. `ble_server` for `esp32_nimble::server::ble_server`.
            let module = record.target().rsplit("::").next().unwrap_or_default();
            let line = format!(
                "{} {module}: {}",
                record.level().as_str().chars().next().unwrap_or('?'),
                record.args()
            );
            let _ = self.flash.try_send((uptime_ms, line));
        }
    }

    fn flush(&self) {}
}

fn write(lines: &Receiver<(u64, String)>) {
    let mut log = match open() {
        Ok(Some(log)) => log,
        Ok(None) => {
            log::warn!(target: SERIAL_ONLY, "no {PARTITION} partition: the event log is off (flash with fw-flash)");
            return;
        }
        Err(err) => {
            log::warn!(target: SERIAL_ONLY, "opening the event log failed: {err}");
            return;
        }
    };
    for (uptime_ms, line) in lines {
        if let Err(err) = log.append(uptime_ms, &line) {
            log::warn!(target: SERIAL_ONLY, "writing the event log failed: {err}");
        }
    }
}

fn open() -> Result<Option<EventLog<Partition>>, EspError> {
    // SAFETY: the only handle to this partition.
    let Some(partition) = (unsafe { EspPartition::new(PARTITION) })? else {
        return Ok(None);
    };
    EventLog::open(Partition(partition)).map(Some)
}

struct Partition(EspPartition);

impl Storage for Partition {
    type Error = EspError;

    fn capacity(&self) -> u32 {
        u32::try_from(self.0.size()).unwrap_or(0)
    }

    fn read(&mut self, offset: u32, buf: &mut [u8]) -> Result<(), EspError> {
        self.0.read(offset as usize, buf)
    }

    fn write(&mut self, offset: u32, data: &[u8]) -> Result<(), EspError> {
        self.0.write(offset as usize, data)
    }

    fn erase_sector(&mut self, offset: u32) -> Result<(), EspError> {
        self.0.erase(offset as usize, SECTOR_SIZE as usize)
    }
}
