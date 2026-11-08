//! Synthetic default and metadata shared with the opt-in video connector.
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

#[derive(Clone, serde::Serialize)]
pub struct SourceStatus {
    pub kind: &'static str,
    pub state: &'static str,
    pub geometry: [u32; 3],
    pub frames: u64,
    pub starts: u64,
    pub failures: u64,
    pub last_error: Option<&'static str>,
}
impl SourceStatus {
    pub fn synthetic() -> Self {
        Self {
            kind: "synthetic",
            state: "running",
            geometry: [16, 16, 3],
            frames: 0,
            starts: 0,
            failures: 0,
            last_error: None,
        }
    }
    pub fn video(width: u32, height: u32) -> Self {
        Self {
            kind: "public-video",
            state: "idle",
            geometry: [width, height, 3],
            frames: 0,
            starts: 0,
            failures: 0,
            last_error: None,
        }
    }
}

pub enum SourceDriver {
    Synthetic {
        source: SyntheticSource,
        status: std::sync::Arc<std::sync::Mutex<SourceStatus>>,
    },
    Video(crate::video::VideoSource),
}
impl SourceDriver {
    pub fn synthetic() -> Self {
        Self::Synthetic {
            source: SyntheticSource::default(),
            status: std::sync::Arc::new(std::sync::Mutex::new(SourceStatus::synthetic())),
        }
    }
    pub fn status(&self) -> std::sync::Arc<std::sync::Mutex<SourceStatus>> {
        match self {
            Self::Synthetic { status, .. } => status.clone(),
            Self::Video(source) => source.status.clone(),
        }
    }
    pub async fn step(
        &mut self,
        engine: &crate::engine::Engine,
        linux_counters: bool,
        stop: &mut tokio::sync::watch::Receiver<bool>,
    ) -> Result<bool, crate::video::VideoError> {
        use crate::{engine::Error, video::VideoError};
        match self {
            Self::Video(source) => {
                source
                    .step_with_counters(
                        engine,
                        move || {
                            if linux_counters {
                                linux_network_counters().map_err(|_| VideoError::Engine)
                            } else {
                                Ok(Zeroizing::new(b"synthetic video counters".to_vec()))
                            }
                        },
                        stop,
                    )
                    .await
            }
            Self::Synthetic { source, status } => {
                if engine.snapshot().map_err(|_| VideoError::Engine)?.queued >= engine.capacity {
                    if let Ok(mut s) = status.lock() {
                        s.state = "backpressure";
                    }
                    return Ok(false);
                }
                let mut sample = source.sample();
                if linux_counters {
                    sample.counters = linux_network_counters().map_err(|_| VideoError::Engine)?;
                }
                match engine.ingest(&sample) {
                    Ok(_) => {
                        if let Ok(mut s) = status.lock() {
                            s.frames += 1;
                            s.state = "running";
                        }
                        Ok(true)
                    }
                    Err(Error::Full) => Ok(false),
                    Err(_) => Err(VideoError::Engine),
                }
            }
        }
    }
    pub async fn shutdown(&mut self) -> Result<(), crate::video::VideoError> {
        if let Self::Video(source) = self {
            source.shutdown().await
        } else {
            Ok(())
        }
    }
    pub async fn run(
        mut self,
        engine: std::sync::Arc<crate::engine::Engine>,
        linux_counters: bool,
        period: std::time::Duration,
        mut stop: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), crate::video::VideoError> {
        use crate::video::VideoError;
        let mut ticker = tokio::time::interval(period);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let outcome = loop {
            if *stop.borrow() {
                break Ok(());
            }
            tokio::select! {_=stop.changed()=>break Ok(()),_=ticker.tick()=>{}}
            if let Err(error) = self.step(&engine, linux_counters, &mut stop).await {
                if error == VideoError::Stopped && *stop.borrow() {
                    break Ok(());
                }
                engine.source_failed();
                if let Ok(mut status) = self.status().lock() {
                    status.state = "failed";
                    status.last_error = Some(error.code());
                }
                break Err(error);
            }
        };
        let cleanup = self.shutdown().await;
        outcome.and(cleanup)
    }
}
