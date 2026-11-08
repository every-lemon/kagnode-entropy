//! Optional bounded metadata ledger. HMAC detects changes only while its key and
//! an independently retained checkpoint remain trusted; this is not root-proof.
use crate::{crypto, engine::Sample, sources::network_totals};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use zeroize::Zeroizing;

const MAX_RECORD_BYTES: usize = 2048;
#[derive(Clone, Copy, Default)]
pub struct Privacy {
    pub timing: bool,
    pub network: bool,
    pub frame_fingerprint: bool,
}
pub struct AuditConfig {
    pub records_per_segment: usize,
    pub retained_segments: usize,
    pub privacy: Privacy,
}
impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            records_per_segment: 1024,
            retained_segments: 4,
            privacy: Privacy::default(),
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case", deny_unknown_fields)]
enum Event {
    Produced {
        sequence: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        duration_ns: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        geometry: Option<[u32; 3]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        frame_fingerprint: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        network_totals: Option<[u64; 2]>,
    },
    Allocated {
        first_sequence: u64,
        count: usize,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    version: u8,
    index: u64,
    previous: String,
    metadata: Event,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    payload: Payload,
    mac: String,
}
#[derive(Serialize, Debug)]
pub struct Checkpoint {
    pub first_retained_index: u64,
    pub index: u64,
    pub head: String,
}
pub struct AuditLog {
    key: Zeroizing<[u8; 32]>,
    dir: PathBuf,
    config: AuditConfig,
    file: File,
    segments: VecDeque<(PathBuf, u64)>,
    segment: u64,
    in_segment: usize,
    index: u64,
    head: [u8; 64],
    failed: bool,
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn private_new(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    options.open(path)
}
pub fn create_key(path: &Path) -> io::Result<()> {
    let mut key = Zeroizing::new([0u8; 32]);
    getrandom::fill(key.as_mut()).map_err(|_| invalid("OS random source unavailable"))?;
    let mut file = private_new(path)?;
    file.write_all(key.as_ref())?;
    file.sync_all()
}
pub fn read_key(path: &Path) -> io::Result<Zeroizing<[u8; 32]>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let mut file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() != 32 {
        return Err(invalid("Audit key must be a 32-byte regular file"));
    }
    #[cfg(unix)]
    if metadata.mode() & 0o077 != 0 || metadata.uid() != rustix::process::geteuid().as_raw() {
        return Err(invalid(
            "Audit key must be owned by this user with mode 0600",
        ));
    }
    let mut key = Zeroizing::new([0u8; 32]);
    file.read_exact(key.as_mut())?;
    Ok(key)
}
impl AuditLog {
    /// Requires a NEW directory. Never deletes existing evidence or resumes a chain.
    pub fn create(dir: &Path, key: Zeroizing<[u8; 32]>, config: AuditConfig) -> io::Result<Self> {
        if !(1..=10000).contains(&config.records_per_segment)
            || !(2..=32).contains(&config.retained_segments)
        {
            return Err(invalid(
                "Audit bounds: 1–10000 records, 2–32 retained segments",
            ));
        }
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        builder.mode(0o700);
        builder.create(dir)?;
        let first = dir.join("segment-00000000000000000000.jsonl");
        let file = private_new(&first)?;
        Ok(Self {
            key,
            dir: dir.to_owned(),
            config,
            file,
            segments: VecDeque::from([(first, 1)]),
            segment: 0,
            in_segment: 0,
            index: 0,
            head: [0u8; 64],
            failed: false,
        })
    }
    pub fn produced(
        &mut self,
        sequence: u64,
        sample: &Sample,
        elapsed: Duration,
    ) -> io::Result<()> {
        let p = self.config.privacy;
        let fingerprint = p.frame_fingerprint.then(|| {
            hex::encode(
                crypto::hmac512(
                    self.key.as_ref(),
                    &crypto::frame(&[crypto::DOMAIN, b"audit-frame", &sample.pixels]),
                )
                .as_ref(),
            )
        });
        self.append(Event::Produced {
            sequence,
            duration_ns: p
                .timing
                .then(|| elapsed.as_nanos().min(u64::MAX as u128) as u64),
            geometry: if p.frame_fingerprint {
                sample.geometry
            } else {
                None
            },
            frame_fingerprint: fingerprint,
            network_totals: if p.network {
                network_totals(&sample.counters)
            } else {
                None
            },
        })
    }
    pub fn allocated(&mut self, first_sequence: u64, count: usize) -> io::Result<()> {
        self.append(Event::Allocated {
            first_sequence,
            count,
        })
    }
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            first_retained_index: self.segments.front().map(|v| v.1).unwrap_or(0),
            index: self.index,
            head: hex::encode(self.head),
        }
    }
    fn append(&mut self, event: Event) -> io::Result<()> {
        if self.failed {
            return Err(invalid("Audit failed; start a new reviewed instance"));
        }
        let result = self.append_inner(event);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn append_inner(&mut self, event: Event) -> io::Result<()> {
        if self.in_segment >= self.config.records_per_segment {
            self.file.sync_data()?;
            self.segment = self
                .segment
                .checked_add(1)
                .ok_or_else(|| invalid("Audit segment limit"))?;
            let path = self.dir.join(format!("segment-{:020}.jsonl", self.segment));
            self.file = private_new(&path)?;
            self.segments.push_back((path, self.index + 1));
            self.in_segment = 0;
            while self.segments.len() > self.config.retained_segments {
                let (old, _) = self.segments.pop_front().expect("segment count");
                fs::remove_file(old)?;
            }
        }
        let index = self
            .index
            .checked_add(1)
            .ok_or_else(|| invalid("Audit index limit"))?;
        let payload = Payload {
            version: 1,
            index,
            previous: hex::encode(self.head),
            metadata: event,
        };
        let encoded = serde_json::to_vec(&payload)?;
        let mac = crypto::hmac512(
            self.key.as_ref(),
            &crypto::frame(&[crypto::DOMAIN, b"audit-record", &encoded]),
        );
        let record = Record {
            payload,
            mac: hex::encode(mac.as_ref()),
        };
        let mut line = serde_json::to_vec(&record)?;
        if line.len() + 1 > MAX_RECORD_BYTES {
            return Err(invalid("Audit record too large"));
        }
        line.push(b'\n');
        self.file.write_all(&line)?;
        self.file.sync_data()?;
        self.head.copy_from_slice(mac.as_ref());
        self.index = index;
        self.in_segment += 1;
        Ok(())
    }
}
/// Verify retained records against an independently saved LAST checkpoint.
/// A matching tail does not prove earlier deleted prefixes or honest telemetry.
pub fn verify(
    dir: &Path,
    key: &[u8],
    expected_index: u64,
    expected_head: &str,
) -> io::Result<Checkpoint> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("segment-") || !name.ends_with(".jsonl") {
            return Err(invalid("Unexpected audit directory entry"));
        }
        if paths.len() >= 32 || !entry.file_type()?.is_file() {
            return Err(invalid("Invalid audit segment"));
        }
        paths.push(entry.path());
    }
    paths.sort();
    let mut previous = None;
    let mut index = None;
    let mut first = 0;
    for path in paths {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW);
        let file = options.open(path)?;
        if file.metadata()?.len() > 20_480_000 {
            return Err(invalid("Audit segment exceeds bounds"));
        }
        let mut reader = BufReader::new(file);
        loop {
            let mut bytes = Vec::new();
            let count = reader
                .by_ref()
                .take((MAX_RECORD_BYTES + 1) as u64)
                .read_until(b'\n', &mut bytes)?;
            if count == 0 {
                break;
            }
            if count > MAX_RECORD_BYTES || bytes.last() != Some(&b'\n') {
                return Err(invalid("Invalid audit record length"));
            }
            let record: Record = serde_json::from_slice(&bytes)?;
            if record.payload.version != 1 {
                return Err(invalid("Unknown audit format"));
            }
            if let Some(last) = index {
                if record.payload.index != last + 1 {
                    return Err(invalid("Audit index gap"));
                }
            } else {
                first = record.payload.index;
            }
            if let Some(ref last) = previous
                && record.payload.previous != *last
            {
                return Err(invalid("Audit chain mismatch"));
            }
            let encoded = serde_json::to_vec(&record.payload)?;
            let expected = crypto::hmac512(
                key,
                &crypto::frame(&[crypto::DOMAIN, b"audit-record", &encoded]),
            );
            let mac = hex::decode(&record.mac).map_err(|_| invalid("Invalid audit MAC"))?;
            use subtle::ConstantTimeEq;
            if !bool::from(expected.as_ref().ct_eq(&mac)) {
                return Err(invalid("Audit MAC mismatch"));
            }
            previous = Some(record.mac);
            index = Some(record.payload.index);
        }
    }
    if index != Some(expected_index) || previous.as_deref() != Some(expected_head) {
        return Err(invalid("Does not match independent checkpoint"));
    }
    Ok(Checkpoint {
        first_retained_index: first,
        index: expected_index,
        head: expected_head.to_owned(),
    })
}
