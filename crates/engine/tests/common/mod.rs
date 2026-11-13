//! Shared helpers for integration tests.

/// A capture line: `<ms> <uuid16> <hex>`.
pub struct Line {
    pub ms: u32,
    pub uuid: u16,
    pub bytes: Vec<u8>,
}

pub fn load(name: &str) -> Vec<Line> {
    let path = format!("{}/tests/fixtures/ftms/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{path}: {e}"))
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let mut parts = l.split_whitespace();
            let ms = parts.next().unwrap().parse().unwrap();
            let uuid = u16::from_str_radix(parts.next().unwrap(), 16).unwrap();
            let hex = parts.next().unwrap();
            let bytes = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            Line { ms, uuid, bytes }
        })
        .collect()
}
