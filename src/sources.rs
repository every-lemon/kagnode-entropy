//! No video URL is fetched. Integrations provide bounded decoded pixels to Sample.
use crate::engine::{MAX_COUNTERS, Sample};
use std::{
    fs::File,
    io::{self, Read},
    time::{SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;
#[derive(Default)]
pub struct SyntheticSource {
    index: u64,
}
impl SyntheticSource {
    pub fn sample(&mut self) -> Sample {
        self.index = self.index.wrapping_add(1);
        Sample {
            pixels: Zeroizing::new(
                (0..768)
                    .map(|i| ((i as u64 + self.index) % 256) as u8)
                    .collect(),
            ),
            timestamp_ns: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            counters: Zeroizing::new(format!("synthetic bytes {}", self.index).into_bytes()),
            geometry: Some([16, 16, 3]),
        }
    }
}
pub fn linux_network_counters() -> io::Result<Zeroizing<Vec<u8>>> {
    let mut bytes = Zeroizing::new(Vec::new());
    File::open("/proc/net/dev")?
        .take((MAX_COUNTERS + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > MAX_COUNTERS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Network counters exceed bounds",
        ));
    }
    Ok(bytes)
}
/// Optional aggregate absolute receive/transmit byte counts; interface names omitted.
pub fn network_totals(bytes: &[u8]) -> Option<[u64; 2]> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut totals = [0u64; 2];
    let mut found = false;
    for line in text.lines().filter(|l| l.contains(':')) {
        let (_, values) = line.split_once(':')?;
        let values: Vec<&str> = values.split_whitespace().collect();
        if values.len() != 16 {
            return None;
        }
        totals[0] = totals[0].checked_add(values[0].parse::<u64>().ok()?)?;
        totals[1] = totals[1].checked_add(values[8].parse::<u64>().ok()?)?;
        found = true;
    }
    found.then_some(totals)
}
