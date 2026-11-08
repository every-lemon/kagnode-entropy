//! Explicit administrator-configured video input. HTTP bytes are fetched by
//! Rust; the decoder gets stdin only, never a URL or shell command.
use crate::{
    engine::{Engine, Error, MAX_FRAME, Sample},
    sources::SourceStatus,
};
use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdout, Command},
    sync::watch,
    task::JoinHandle,
};
use zeroize::Zeroizing;

pub type Operation<'a, T> = Pin<Box<dyn Future<Output = Result<T, VideoError>> + Send + 'a>>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoError {
    Configuration,
    Network,
    InputLimit,
    Spawn,
    EndOfStream,
    TruncatedFrame,
    FrameTimeout,
    Stopped,
    Reap,
    Engine,
}
impl VideoError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Configuration => "video-configuration",
            Self::Network => "video-network",
            Self::InputLimit => "video-input-limit",
            Self::Spawn => "decoder-start",
            Self::EndOfStream => "video-ended",
            Self::TruncatedFrame => "frame-truncated",
            Self::FrameTimeout => "frame-timeout",
            Self::Stopped => "source-stopped",
            Self::Reap => "decoder-cleanup",
            Self::Engine => "engine-failure",
        }
    }
}
impl std::fmt::Display for VideoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}
impl std::error::Error for VideoError {}

#[derive(Clone)]
pub struct VideoConfig {
    pub url: String,
    pub decoder: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub frame_timeout: Duration,
    pub session_lifetime: Duration,
    pub max_input_bytes: u64,
    pub memory_mib: u64,
    pub retries: u8,
    pub retry_delay: Duration,
}
impl VideoConfig {
    pub fn validate(&self) -> Result<(), VideoError> {
        crate::video_http::validate_url(&self.url)?;
        if !self.decoder.is_absolute()
            || !(1..=512).contains(&self.width)
            || !(1..=512).contains(&self.height)
            || self.frame_bytes() > MAX_FRAME
            || !(1..=30).contains(&self.fps)
            || !(Duration::from_millis(100)..=Duration::from_secs(30)).contains(&self.frame_timeout)
            || !(Duration::from_secs(1)..=Duration::from_secs(600)).contains(&self.session_lifetime)
            || !(1024..=256 * 1024 * 1024).contains(&self.max_input_bytes)
            || !(128..=2048).contains(&self.memory_mib)
            || self.retries > 5
            || !(Duration::from_millis(100)..=Duration::from_secs(30)).contains(&self.retry_delay)
        {
            return Err(VideoError::Configuration);
        }
        Ok(())
    }
    pub fn frame_bytes(&self) -> usize {
        (self.width as usize)
            .saturating_mul(self.height as usize)
            .saturating_mul(3)
    }
    pub fn decoder_args(&self) -> Vec<String> {
        [
            "-hide_banner",
            "-loglevel",
            "quiet",
            "-nostats",
            "-nostdin",
            "-max_alloc",
            "16777216",
            "-threads",
            "1",
            "-filter_threads",
            "1",
            "-filter_complex_threads",
            "1",
            "-probesize",
            "1048576",
            "-analyzeduration",
            "3000000",
            "-protocol_whitelist",
            "pipe",
            "-format_whitelist",
            "mov,matroska,webm,mpegts,mpeg,avi,ogg,flv,h264",
            "-i",
            "pipe:0",
            "-map",
            "0:v:0",
            "-an",
            "-sn",
            "-dn",
            "-vf",
            &format!(
                "fps={},scale={}:{}:flags=bilinear",
                self.fps, self.width, self.height
            ),
            "-pix_fmt",
            "rgb24",
            "-c:v",
            "rawvideo",
            "-threads:v",
            "1",
            "-f",
            "rawvideo",
            "pipe:1",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }
}
pub trait EncodedInput: Send {
    fn next_chunk(&mut self) -> Operation<'_, Option<bytes::Bytes>>;
}
pub trait VideoTransport: Send + Sync {
    fn open<'a>(&'a self, config: &'a VideoConfig) -> Operation<'a, Box<dyn EncodedInput>>;
}
pub trait DecoderSession: Send {
    fn read_frame<'a>(&'a mut self, buffer: &'a mut [u8]) -> Operation<'a, ()>;
    fn stop(&mut self) -> Operation<'_, ()>;
    fn process_id(&self) -> Option<u32> {
        None
    }
}
pub trait DecoderFactory: Send + Sync {
    fn start<'a>(&'a self, config: &'a VideoConfig) -> Operation<'a, Box<dyn DecoderSession>>;
}

pub struct FfmpegFactory {
    pub transport: Arc<dyn VideoTransport>,
}
impl DecoderFactory for FfmpegFactory {
    fn start<'a>(&'a self, config: &'a VideoConfig) -> Operation<'a, Box<dyn DecoderSession>> {
        Box::pin(async move {
            config.validate()?;
            // Reject paths which are not local regular executable files. Administrator
            // ownership/trust is still required; no arbitrary remote binary download.
            validate_decoder_path(&config.decoder)?;
            let mut input = self.transport.open(config).await?;
            let mut command = decoder_command(config)?;
            let mut child = command.spawn().map_err(|_| VideoError::Spawn)?;
            let stdout = child.stdout.take().ok_or(VideoError::Spawn)?;
            let mut stdin = child.stdin.take().ok_or(VideoError::Spawn)?;
            let max_bytes = config.max_input_bytes;
            let idle = config.frame_timeout;
            let feeder = tokio::spawn(async move {
                let mut total = 0u64;
                loop {
                    let chunk = tokio::time::timeout(idle, input.next_chunk())
                        .await
                        .map_err(|_| VideoError::Network)??;
                    let Some(chunk) = chunk else {
                        stdin
                            .shutdown()
                            .await
                            .map_err(|_| VideoError::EndOfStream)?;
                        return Ok(());
                    };
                    total = total
                        .checked_add(chunk.len() as u64)
                        .ok_or(VideoError::InputLimit)?;
                    if total > max_bytes {
                        return Err(VideoError::InputLimit);
                    }
                    // Backpressure propagates through both OS pipes; no media queue
                    // or complete response body is accumulated in the parent.
                    for part in chunk.chunks(65536) {
                        stdin
                            .write_all(part)
                            .await
                            .map_err(|_| VideoError::EndOfStream)?;
                    }
                }
            });
            Ok(Box::new(FfmpegSession {
                child,
                stdout,
                feeder: Some(feeder),
                stopped: false,
            }) as Box<dyn DecoderSession>)
        })
    }
}
pub fn validate_decoder_path(path: &Path) -> Result<(), VideoError> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::metadata(path).map_err(|_| VideoError::Configuration)?;
    if !path.is_absolute() || !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(VideoError::Configuration);
    }
    Ok(())
}
/// Build only: this does not spawn the decoder or fetch any URL.
pub fn decoder_command(config: &VideoConfig) -> Result<Command, VideoError> {
    config.validate()?;
    let mut command = Command::new(&config.decoder);
    command
        .args(config.decoder_args())
        .env_clear()
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .current_dir("/")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .process_group(0);
    let address_space = config.memory_mib * 1024 * 1024;
    // SAFETY: these child-only pre-exec calls use async-signal-safe libc calls,
    // fixed stack values and no locks/allocations before exec. They do not modify
    // the parent process or system-wide resource policy.
    unsafe {
        command.pre_exec(move || {
            for (resource, limit) in [
                (libc::RLIMIT_AS, address_space),
                (libc::RLIMIT_CORE, 0),
                (libc::RLIMIT_FSIZE, 0),
                (libc::RLIMIT_NOFILE, 64),
                (libc::RLIMIT_CPU, 120),
            ] {
                let value = libc::rlimit {
                    rlim_cur: limit as libc::rlim_t,
                    rlim_max: limit as libc::rlim_t,
                };
                if libc::setrlimit(resource, &value) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    Ok(command)
}
struct FfmpegSession {
    child: Child,
    stdout: ChildStdout,
    feeder: Option<JoinHandle<Result<(), VideoError>>>,
    stopped: bool,
}
impl FfmpegSession {
    fn kill_group(&mut self) {
        if let Some(id) = self.child.id() {
            // SAFETY: process_group(0) made this owned child's PID its group ID.
            // The unreaped child still owns that PID; no arbitrary process is selected.
            unsafe {
                libc::kill(-(id as i32), libc::SIGKILL);
            }
            let _ = self.child.start_kill();
        }
    }
}
impl DecoderSession for FfmpegSession {
    fn process_id(&self) -> Option<u32> {
        self.child.id()
    }
    fn read_frame<'a>(&'a mut self, buffer: &'a mut [u8]) -> Operation<'a, ()> {
        Box::pin(async move {
            let mut offset = 0;
            while offset < buffer.len() {
                let n = self
                    .stdout
                    .read(&mut buffer[offset..])
                    .await
                    .map_err(|_| VideoError::TruncatedFrame)?;
                if n == 0 {
                    if self.feeder.as_ref().is_some_and(|t| t.is_finished()) {
                        let task = self.feeder.take().expect("present feeder");
                        if let Ok(Err(error)) = task.await {
                            return Err(error);
                        }
                    }
                    return Err(if offset == 0 {
                        VideoError::EndOfStream
                    } else {
                        VideoError::TruncatedFrame
                    });
                }
                offset += n;
            }
            Ok(())
        })
    }
    fn stop(&mut self) -> Operation<'_, ()> {
        Box::pin(async move {
            if self.stopped {
                return Ok(());
            }
            if let Some(task) = self.feeder.take() {
                task.abort();
                let _ = task.await;
            }
            self.kill_group();
            tokio::time::timeout(Duration::from_secs(5), self.child.wait())
                .await
                .map_err(|_| VideoError::Reap)?
                .map_err(|_| VideoError::Reap)?;
            self.stopped = true;
            Ok(())
        })
    }
}
impl Drop for FfmpegSession {
    fn drop(&mut self) {
        if let Some(task) = self.feeder.take() {
            task.abort();
        }
        if !self.stopped {
            self.kill_group();
        }
    }
}

pub struct VideoSource {
    config: VideoConfig,
    factory: Arc<dyn DecoderFactory>,
    session: Option<Box<dyn DecoderSession>>,
    started: Option<Instant>,
    session_frames: u64,
    failures: u8,
    exhausted: bool,
    pub status: Arc<Mutex<SourceStatus>>,
    clock: Arc<dyn Fn() -> u128 + Send + Sync>,
}
impl VideoSource {
    pub fn new(config: VideoConfig, factory: Arc<dyn DecoderFactory>) -> Result<Self, VideoError> {
        Self::with_clock(
            config,
            factory,
            Arc::new(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            }),
        )
    }
    /// Deterministic clock injection for source-adapter tests. Clock values are
    /// supplemental input and never asserted to carry entropy.
    pub fn with_clock(
        config: VideoConfig,
        factory: Arc<dyn DecoderFactory>,
        clock: Arc<dyn Fn() -> u128 + Send + Sync>,
    ) -> Result<Self, VideoError> {
        config.validate()?;
        let status = Arc::new(Mutex::new(SourceStatus::video(config.width, config.height)));
        Ok(Self {
            config,
            factory,
            session: None,
            started: None,
            session_frames: 0,
            failures: 0,
            exhausted: false,
            status,
            clock,
        })
    }
    fn state(&self, state: &'static str, error: Option<VideoError>) {
        if let Ok(mut s) = self.status.lock() {
            s.state = state;
            s.last_error = error.map(VideoError::code);
        }
    }
    async fn stop_session(&mut self) -> Result<(), VideoError> {
        if let Some(mut session) = self.session.take()
            && let Err(error) = session.stop().await
        {
            self.exhausted = true;
            self.state("failed", Some(error));
            return Err(error);
        }
        self.started = None;
        self.session_frames = 0;
        Ok(())
    }
    pub async fn shutdown(&mut self) -> Result<(), VideoError> {
        let result = self.stop_session().await;
        if result.is_ok() && self.status.lock().is_ok_and(|s| s.state != "failed") {
            self.state("stopped", None);
        }
        result
    }
    /// One bounded read and ingest. Returns false on backpressure or a scheduled
    /// retry; never falls back to synthetic pixels after a video failure.
    pub async fn step(
        &mut self,
        engine: &Engine,
        counters: Zeroizing<Vec<u8>>,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<bool, VideoError> {
        self.step_with_counters(engine, || Ok(counters), stop).await
    }
    pub async fn step_with_counters<F>(
        &mut self,
        engine: &Engine,
        get_counters: F,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<bool, VideoError>
    where
        F: FnOnce() -> Result<Zeroizing<Vec<u8>>, VideoError> + Send,
    {
        if self.exhausted {
            return Err(VideoError::Stopped);
        }
        if *stop.borrow() {
            self.shutdown().await?;
            return Err(VideoError::Stopped);
        }
        let expired = self
            .started
            .is_some_and(|t| t.elapsed() >= self.config.session_lifetime)
            || self.session_frames
                >= self.config.fps as u64 * self.config.session_lifetime.as_secs();
        if expired {
            self.stop_session().await?;
        }
        let queued = match engine.snapshot() {
            Ok(snapshot) => snapshot.queued,
            Err(_) => {
                self.stop_session().await?;
                self.exhausted = true;
                self.state("failed", Some(VideoError::Engine));
                return Err(VideoError::Engine);
            }
        };
        if queued >= engine.capacity {
            self.state("backpressure", None);
            return Ok(false);
        }
        if self.session.is_none() {
            self.state("connecting", None);
            let opened = tokio::select! {
                _=stop.changed()=>return Err(VideoError::Stopped),
                result=tokio::time::timeout(self.config.frame_timeout,self.factory.start(&self.config))=>result.map_err(|_|VideoError::FrameTimeout).and_then(|r|r),
            };
            match opened {
                Ok(session) => {
                    self.session = Some(session);
                    self.started = Some(Instant::now());
                    if let Ok(mut s) = self.status.lock() {
                        s.starts += 1;
                    }
                }
                Err(error) => return self.failure(engine, error, stop).await,
            }
        }
        let mut pixels = Zeroizing::new(vec![0u8; self.config.frame_bytes()]);
        let session = self.session.as_mut().expect("started session");
        let remaining = self
            .config
            .session_lifetime
            .saturating_sub(self.started.expect("started session").elapsed());
        let read = tokio::select! {
            _=stop.changed()=>Err(VideoError::Stopped),
            result=tokio::time::timeout(self.config.frame_timeout.min(remaining),session.read_frame(&mut pixels))=>result.map_err(|_|VideoError::FrameTimeout).and_then(|r|r),
        };
        if let Err(error) = read {
            let renewal = (error == VideoError::FrameTimeout
                && self
                    .started
                    .is_some_and(|t| t.elapsed() >= self.config.session_lifetime))
                || (error == VideoError::InputLimit && self.session_frames > 0);
            self.stop_session().await?;
            if renewal {
                self.state("restarting", None);
                return Ok(false);
            }
            if error == VideoError::Stopped {
                return Err(error);
            }
            return self.failure(engine, error, stop).await;
        }
        self.session_frames += 1;
        let timestamp_ns = (self.clock)();
        let counters = match get_counters() {
            Ok(counters) => counters,
            Err(error) => {
                self.stop_session().await?;
                self.exhausted = true;
                self.state("failed", Some(error));
                engine.source_failed();
                return Err(error);
            }
        };
        let sample = Sample {
            pixels,
            timestamp_ns,
            counters,
            geometry: Some([self.config.width, self.config.height, 3]),
        };
        match engine.ingest(&sample) {
            Ok(_) => {
                self.state("running", None);
                if let Ok(mut s) = self.status.lock() {
                    s.frames += 1;
                }
                Ok(true)
            }
            Err(Error::Full) => {
                self.state("backpressure", None);
                Ok(false)
            }
            Err(_) => {
                self.stop_session().await?;
                self.exhausted = true;
                self.state("failed", Some(VideoError::Engine));
                Err(VideoError::Engine)
            }
        }
    }
    async fn failure(
        &mut self,
        engine: &Engine,
        error: VideoError,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<bool, VideoError> {
        engine.source_failed();
        self.failures = self.failures.saturating_add(1);
        if let Ok(mut s) = self.status.lock() {
            s.failures += 1;
        }
        if self.failures > self.config.retries {
            self.exhausted = true;
            self.state("failed", Some(error));
            return Err(error);
        }
        self.state("retrying", Some(error));
        tokio::select! {_=stop.changed()=>Err(VideoError::Stopped),_=tokio::time::sleep(self.config.retry_delay)=>Ok(false)}
    }
}
