//! A persistent log: a ring of log lines in a flash partition, kept across reboots so a run can be
//! checked afterwards.
//!
//! # Layout
//!
//! The partition is split into [`SECTOR_SIZE`] sectors of [`SLOT_SIZE`] slots. Each slot holds
//! one [`Entry`]: a format byte, the payload length, the postcard payload and a CRC-16. An erased
//! slot is all `0xff`. Entries carry a sequence number that keeps counting across reboots, and
//! lines longer than [`MAX_TEXT`] are cut.
//!
//! - **Writing** fills slots in order. Entering a sector erases it first, which drops the oldest
//!   entries, so the log always holds the newest ones.
//! - **Opening** scans every slot for the highest sequence number and continues after it. A slot
//!   left half-written by a power cut fails its CRC and is skipped.
//! - **Reading** starts at the write position, so it returns the oldest entry first.
//!
//! The flash itself sits behind [`Storage`], so this runs on the host against [`MemStorage`].

use serde::{Deserialize, Serialize};

/// Flash erase unit.
pub const SECTOR_SIZE: u32 = 4096;
/// One entry per slot.
pub const SLOT_SIZE: u32 = 128;
const SLOTS_PER_SECTOR: u32 = SECTOR_SIZE / SLOT_SIZE;

/// Bumped on any change that old entries would decode wrongly under.
const FORMAT: u8 = 2;
/// Format byte, length byte, payload, CRC-16.
const MAX_PAYLOAD: usize = SLOT_SIZE as usize - 4;
/// The longest line kept: the payload less the largest seq, uptime and text length varints.
pub const MAX_TEXT: usize = MAX_PAYLOAD - 5 - 10 - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry<'a> {
    /// Counts up across reboots; orders the log.
    pub seq: u32,
    /// Milliseconds since this boot.
    pub uptime_ms: u64,
    pub text: &'a str,
}

/// A flash partition: byte-addressed, written only where erased, erased by whole sectors.
pub trait Storage {
    type Error;

    /// Size in bytes: a whole number of sectors.
    fn capacity(&self) -> u32;

    /// # Errors
    /// Whatever the flash driver reports.
    fn read(&mut self, offset: u32, buf: &mut [u8]) -> Result<(), Self::Error>;

    /// Writes into erased bytes.
    ///
    /// # Errors
    /// Whatever the flash driver reports.
    fn write(&mut self, offset: u32, data: &[u8]) -> Result<(), Self::Error>;

    /// Erases one sector, setting it to `0xff`. `offset` is sector-aligned.
    ///
    /// # Errors
    /// Whatever the flash driver reports.
    fn erase_sector(&mut self, offset: u32) -> Result<(), Self::Error>;
}

/// Storage in RAM, behaving like NOR flash: writes can only clear bits. For tests, and for
/// decoding a partition read off the device.
pub struct MemStorage<'a>(pub &'a mut [u8]);

impl Storage for MemStorage<'_> {
    type Error = core::convert::Infallible;

    fn capacity(&self) -> u32 {
        u32::try_from(self.0.len()).unwrap_or(u32::MAX) / SECTOR_SIZE * SECTOR_SIZE
    }

    fn read(&mut self, offset: u32, buf: &mut [u8]) -> Result<(), Self::Error> {
        let start = offset as usize;
        buf.copy_from_slice(&self.0[start..start + buf.len()]);
        Ok(())
    }

    fn write(&mut self, offset: u32, data: &[u8]) -> Result<(), Self::Error> {
        let start = offset as usize;
        for (cell, byte) in self.0[start..start + data.len()].iter_mut().zip(data) {
            *cell &= byte;
        }
        Ok(())
    }

    fn erase_sector(&mut self, offset: u32) -> Result<(), Self::Error> {
        let start = offset as usize;
        self.0[start..start + SECTOR_SIZE as usize].fill(0xff);
        Ok(())
    }
}

pub struct EventLog<S> {
    storage: S,
    slots: u32,
    /// The slot the next entry goes in.
    next_slot: u32,
    next_seq: u32,
}

impl<S: Storage> EventLog<S> {
    /// Scans the storage and continues after its newest entry. Writes nothing.
    ///
    /// # Errors
    /// A failed read.
    ///
    /// # Panics
    /// If the storage is smaller than one sector.
    pub fn open(mut storage: S) -> Result<Self, S::Error> {
        let slots = storage.capacity() / SLOT_SIZE;
        assert!(
            slots >= SLOTS_PER_SECTOR,
            "event log storage under one sector"
        );

        let mut newest: Option<(u32, u32)> = None;
        let mut buf = [0; SLOT_SIZE as usize];
        for slot in 0..slots {
            if let Some(entry) = read_slot(&mut storage, slot, &mut buf)?
                && newest.is_none_or(|(_, seq)| entry.seq > seq)
            {
                newest = Some((slot, entry.seq));
            }
        }

        let (next_slot, next_seq) = match newest {
            None => (0, 0),
            Some((slot, seq)) => {
                let mut next = (slot + 1) % slots;
                // A half-written slot after the newest entry can't be written again; skip to the
                // next erased one, or to the next sector, which gets erased.
                while !next.is_multiple_of(SLOTS_PER_SECTOR) && !slot_erased(&mut storage, next)? {
                    next = (next + 1) % slots;
                }
                (next, seq.wrapping_add(1))
            }
        };
        Ok(Self {
            storage,
            slots,
            next_slot,
            next_seq,
        })
    }

    /// Appends a line, cut to [`MAX_TEXT`] bytes, erasing the oldest sector first when the log
    /// wraps into it.
    ///
    /// # Errors
    /// A failed erase or write. The entry is then lost, and the next one goes in the slot after.
    pub fn append(&mut self, uptime_ms: u64, text: &str) -> Result<(), S::Error> {
        let entry = Entry {
            seq: self.next_seq,
            uptime_ms,
            text: &text[..text.floor_char_boundary(MAX_TEXT)],
        };
        let slot = self.next_slot;
        self.next_slot = (slot + 1) % self.slots;
        self.next_seq = self.next_seq.wrapping_add(1);

        if slot.is_multiple_of(SLOTS_PER_SECTOR) {
            self.storage.erase_sector(slot * SLOT_SIZE)?;
        }
        self.storage.write(slot * SLOT_SIZE, &encode(&entry))
    }

    /// Calls `f` with every entry, oldest first.
    ///
    /// # Errors
    /// A failed read.
    pub fn for_each(&mut self, mut f: impl FnMut(Entry<'_>)) -> Result<(), S::Error> {
        let mut buf = [0; SLOT_SIZE as usize];
        for i in 0..self.slots {
            let slot = (self.next_slot + i) % self.slots;
            if let Some(entry) = read_slot(&mut self.storage, slot, &mut buf)? {
                f(entry);
            }
        }
        Ok(())
    }
}

fn read_slot<'a, S: Storage>(
    storage: &mut S,
    slot: u32,
    buf: &'a mut [u8; SLOT_SIZE as usize],
) -> Result<Option<Entry<'a>>, S::Error> {
    storage.read(slot * SLOT_SIZE, buf)?;
    Ok(decode(buf))
}

fn slot_erased<S: Storage>(storage: &mut S, slot: u32) -> Result<bool, S::Error> {
    let mut buf = [0; SLOT_SIZE as usize];
    storage.read(slot * SLOT_SIZE, &mut buf)?;
    Ok(buf.iter().all(|&b| b == 0xff))
}

fn encode(entry: &Entry) -> [u8; SLOT_SIZE as usize] {
    let mut slot = [0xff; SLOT_SIZE as usize];
    let len = postcard::to_slice(entry, &mut slot[2..2 + MAX_PAYLOAD])
        .expect("every entry fits a slot")
        .len();
    slot[0] = FORMAT;
    #[allow(clippy::cast_possible_truncation)] // at most MAX_PAYLOAD
    {
        slot[1] = len as u8;
    }
    let crc = crc16(&slot[..2 + len]);
    slot[2 + len..4 + len].copy_from_slice(&crc.to_le_bytes());
    slot
}

fn decode(slot: &[u8; SLOT_SIZE as usize]) -> Option<Entry<'_>> {
    let len = usize::from(slot[1]);
    if slot[0] != FORMAT || len > MAX_PAYLOAD {
        return None;
    }
    let crc = u16::from_le_bytes([slot[2 + len], slot[3 + len]]);
    if crc != crc16(&slot[..2 + len]) {
        return None;
    }
    postcard::from_bytes(&slot[2..2 + len]).ok()
}

/// CRC-16/CCITT-FALSE.
fn crc16(bytes: &[u8]) -> u16 {
    let mut crc: u16 = 0xffff;
    for &byte in bytes {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 == 0 {
                crc << 1
            } else {
                (crc << 1) ^ 0x1021
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_the_standard_check_value() {
        assert_eq!(crc16(b"123456789"), 0x29b1);
    }

    #[test]
    fn the_largest_entry_fits_a_slot() {
        let entry = Entry {
            seq: u32::MAX,
            uptime_ms: u64::MAX,
            text: &"x".repeat(MAX_TEXT),
        };
        assert_eq!(decode(&encode(&entry)), Some(entry));
    }

    #[test]
    fn erased_and_corrupt_slots_decode_to_nothing() {
        assert_eq!(decode(&[0xff; SLOT_SIZE as usize]), None);
        let mut slot = encode(&Entry {
            seq: 7,
            uptime_ms: 1234,
            text: "watch connected",
        });
        slot[3] ^= 0x01;
        assert_eq!(decode(&slot), None);
    }
}
