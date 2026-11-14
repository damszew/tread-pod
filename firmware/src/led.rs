//! The XIAO ESP32-S3 user LED (yellow, GPIO21, lit when driven low) shows the treadmill link.
//!
//! The watch shows its own connection to the footpod, so the LED only covers the treadmill side:
//!
//! | LED | meaning |
//! |---|---|
//! | off | no watch listening, so no treadmill wanted |
//! | slow blink (1 Hz) | looking for a treadmill |
//! | fast blink (5 Hz) | connecting, or connected and waiting for the first data |
//! | on | forwarding treadmill data to the watch |

use engine::session::Link;
use esp_idf_svc::hal::gpio::{Output, OutputPin, PinDriver};

pub struct Led {
    pin: PinDriver<'static, Output>,
    lit: Option<bool>,
}

impl Led {
    pub fn new(pin: impl OutputPin + 'static) -> anyhow::Result<Self> {
        let mut led = Self {
            pin: PinDriver::output(pin)?,
            lit: None,
        };
        led.set(false);
        Ok(led)
    }

    /// Updates the LED for the link state at `now_ms`; call at least every 100 ms.
    pub fn show(&mut self, link: Link, now_ms: u64) {
        let lit = match link {
            Link::Idle => false,
            Link::Scanning { .. } => blink(now_ms, 1_000),
            Link::Connecting { .. }
            | Link::Connected {
                last_data: None, ..
            } => blink(now_ms, 200),
            Link::Connected { .. } => true,
        };
        self.set(lit);
    }

    fn set(&mut self, lit: bool) {
        if self.lit == Some(lit) {
            return;
        }
        // Active low.
        let result = if lit {
            self.pin.set_low()
        } else {
            self.pin.set_high()
        };
        match result {
            Ok(()) => self.lit = Some(lit),
            Err(err) => log::warn!("LED: {err}"),
        }
    }
}

/// On for the first half of every `period_ms`.
fn blink(now_ms: u64, period_ms: u64) -> bool {
    now_ms % period_ms < period_ms / 2
}
