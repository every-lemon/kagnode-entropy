use crate::{
    audit::AuditLog,
    crypto::{self, RandomSource, Secret},
};
use hkdf::Hkdf;
use serde::Serialize;
use sha2::Sha512;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard},
    time::Instant,
};
use zeroize::{Zeroize, Zeroizing};

pub const MAX_FRAME: usize = 1024 * 1024;
pub const MAX_COUNTERS: usize = 16384;
pub const MAX_BATCH: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    OsBacked,
    Experimental,
}
impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::OsBacked => "os-backed",
            Self::Experimental => "experimental",
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Full,
    Empty,
    Source,
    Audit,
    Closed,
    Forked,
    Poisoned,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}

pub struct Sample {
    pub pixels: Zeroizing<Vec<u8>>,
    pub timestamp_ns: u128,
    pub counters: Zeroizing<Vec<u8>>,
    pub geometry: Option<[u32; 3]>,
}
impl Sample {
    pub fn validate(&self) -> Result<(), Error> {
        if self.pixels.is_empty()
            || self.pixels.len() > MAX_FRAME
            || self.counters.is_empty()
            || self.counters.len() > MAX_COUNTERS
        {
            return Err(Error::Invalid);
        }
        if let Some(g) = self.geometry {
            let size = g.iter().try_fold(1usize, |n, &v| n.checked_mul(v as usize));
            if g.contains(&0) || size != Some(self.pixels.len()) {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
}
struct Chunk {
    sequence: u64,
    material: Secret,
}
struct State {
    key: Secret,
    queue: VecDeque<Chunk>,
    produced: u64,
    consumed: u64,
    source_failures: u64,
    fault: bool,
    closed: bool,
    audit: Option<AuditLog>,
}
pub struct Engine {
    state: Mutex<State>,
    random: Arc<dyn RandomSource>,
    pid: u32,
    pub mode: Mode,
    pub capacity: usize,
    pub instance: String,
}
#[derive(Serialize)]
pub struct Snapshot {
    pub instance: String,
    pub mode: Mode,
    pub capacity: usize,
    pub queued: usize,
    pub produced: u64,
    pub consumed: u64,
    pub source_failures: u64,
    pub producer_fault: bool,
    pub supplemental_entropy_bits_claimed: u8,
    pub audit_enabled: bool,
    pub trusted_execution: &'static str,
}
pub struct Delivery {
    pub sequence: u64,
    pub value: Secret,
}
impl Engine {
    pub fn new(
        mode: Mode,
        capacity: usize,
        random: Arc<dyn RandomSource>,
        audit: Option<AuditLog>,
    ) -> Result<Self, Error> {
        if !(1..=4096).contains(&capacity) {
            return Err(Error::Invalid);
        }
        let mut key = Zeroizing::new([0u8; 64]);
        random.fill(key.as_mut()).map_err(|_| Error::Source)?;
        let mut instance = [0u8; 16];
        random.fill(&mut instance).map_err(|_| Error::Source)?;
        Ok(Self {
            state: Mutex::new(State {
                key,
                queue: VecDeque::with_capacity(capacity),
                produced: 0,
                consumed: 0,
                source_failures: 0,
                fault: false,
                closed: false,
                audit,
            }),
            random,
            pid: std::process::id(),
            mode,
            capacity,
            instance: hex::encode(instance),
        })
    }
    fn lock(&self) -> Result<MutexGuard<'_, State>, Error> {
        // Test process identity BEFORE touching a mutex potentially inherited locked.
        if self.pid != std::process::id() {
            return Err(Error::Forked);
        }
        let state = self.state.lock().map_err(|_| Error::Poisoned)?;
        if state.closed {
            return Err(Error::Closed);
        }
        Ok(state)
    }
    pub fn ingest(&self, sample: &Sample) -> Result<u64, Error> {
        sample.validate()?;
        let start = Instant::now();
        let mut state = self.lock()?;
        if state.queue.len() >= self.capacity {
            return Err(Error::Full);
        }
        let sequence = state.produced.checked_add(1).ok_or(Error::Closed)?;
        let mut fresh = Zeroizing::new([0u8; 64]);
        if self.mode == Mode::OsBacked && self.random.fill(fresh.as_mut()).is_err() {
            state.fault = true;
            state.source_failures = state.source_failures.saturating_add(1);
            return Err(Error::Source);
        }
        // Production bytes remain exactly the OS CSPRNG output. Public samples,
        // identities and the experimental feedback construction cannot weaken it.
        let (chunk, next_key) = if self.mode == Mode::OsBacked {
            (fresh, Zeroizing::new([0u8; 64]))
        } else {
            let s3 = crypto::blueprint(
                state.key.as_ref(),
                &sample.pixels,
                sample.timestamp_ns,
                &sample.counters,
            );
            let ikm = crypto::frame(&[
                crypto::DOMAIN,
                self.mode.label().as_bytes(),
                s3.as_ref(),
                &sequence.to_be_bytes(),
            ]);
            let hk = Hkdf::<Sha512>::new(Some(state.key.as_ref()), &ikm);
            let mut chunk = Zeroizing::new([0u8; 64]);
            let mut next_key = Zeroizing::new([0u8; 64]);
            hk.expand(&[crypto::DOMAIN, b"pool-chunk"].concat(), chunk.as_mut())
                .expect("64-byte HKDF output");
            hk.expand(&[crypto::DOMAIN, b"next-key"].concat(), next_key.as_mut())
                .expect("64-byte HKDF output");
            (chunk, next_key)
        };
        if let Some(audit) = state.audit.as_mut()
            && audit.produced(sequence, sample, start.elapsed()).is_err()
        {
            state.fault = true;
            return Err(Error::Audit);
        }
        state.key.zeroize();
        state.key = next_key;
        state.queue.push_back(Chunk {
            sequence,
            material: chunk,
        });
        state.produced = sequence;
        state.fault = false;
        Ok(sequence)
    }
    pub fn consume(
        &self,
        client: &str,
        count: usize,
        context: &str,
    ) -> Result<Vec<Delivery>, Error> {
        if client.is_empty()
            || client.len() > 128
            || context.len() > 128
            || !(1..=MAX_BATCH).contains(&count)
        {
            return Err(Error::Invalid);
        }
        let mut state = self.lock()?;
        if state.fault {
            return Err(Error::Source);
        }
        if state.queue.len() < count {
            return Err(Error::Empty);
        }
        let mut result = Vec::with_capacity(count);
        for _ in 0..count {
            let chunk = state
                .queue
                .pop_front()
                .expect("batch length checked under lock");
            state.consumed += 1;
            let fields = crypto::frame(&[
                crypto::DOMAIN,
                b"client-output",
                client.as_bytes(),
                context.as_bytes(),
                self.instance.as_bytes(),
                &chunk.sequence.to_be_bytes(),
            ]);
            let value = if self.mode == Mode::OsBacked {
                chunk.material
            } else {
                crypto::hmac512(chunk.material.as_ref(), &fields)
            };
            result.push(Delivery {
                sequence: chunk.sequence,
                value,
            });
        }
        if let Some(audit) = state.audit.as_mut()
            && audit.allocated(result[0].sequence, count).is_err()
        {
            state.fault = true;
            return Err(Error::Audit);
        }
        Ok(result)
    }
    pub fn snapshot(&self) -> Result<Snapshot, Error> {
        let s = self.lock()?;
        Ok(Snapshot {
            instance: self.instance.clone(),
            mode: self.mode,
            capacity: self.capacity,
            queued: s.queue.len(),
            produced: s.produced,
            consumed: s.consumed,
            source_failures: s.source_failures,
            producer_fault: s.fault,
            supplemental_entropy_bits_claimed: 0,
            audit_enabled: s.audit.is_some(),
            trusted_execution: "none: ordinary process",
        })
    }
    pub fn audit_checkpoint(&self) -> Option<crate::audit::Checkpoint> {
        if self.pid != std::process::id() {
            return None;
        }
        self.state
            .lock()
            .ok()?
            .audit
            .as_ref()
            .map(|a| a.checkpoint())
    }
    pub fn source_failed(&self) {
        if let Ok(mut s) = self.lock() {
            s.fault = true;
            s.source_failures = s.source_failures.saturating_add(1);
        }
    }
    /// Zeroizes owned key/queue allocations. Does not erase registers, sockets, copies or swap.
    pub fn close(&self) {
        if self.pid != std::process::id() {
            return;
        }
        if let Ok(mut s) = self.state.lock() {
            s.closed = true;
            s.key.zeroize();
            s.queue.clear();
        }
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixed;
    impl RandomSource for Fixed {
        fn fill(&self, out: &mut [u8]) -> Result<(), &'static str> {
            out.fill(7);
            Ok(())
        }
    }
    #[test]
    fn fork_rejected_before_inherited_lock() {
        let mut e = Engine::new(Mode::OsBacked, 1, Arc::new(Fixed), None).unwrap();
        e.pid = e.pid.wrapping_add(1);
        let _held = e.state.lock().unwrap();
        assert!(matches!(e.snapshot(), Err(Error::Forked)));
    }
    #[test]
    fn independent_key_and_chunk_and_owned_zeroization() {
        let e = Engine::new(Mode::OsBacked, 1, Arc::new(Fixed), None).unwrap();
        let sample = crate::sources::SyntheticSource::default().sample();
        e.ingest(&sample).unwrap();
        {
            let state = e.state.lock().unwrap();
            assert_ne!(state.key, state.queue[0].material);
        }
        e.close();
        let state = e.state.lock().unwrap();
        assert!(state.key.iter().all(|b| *b == 0));
        assert!(state.queue.is_empty());
    }
}
