//! No network request or FFmpeg execution occurs. Fixtures use mocked transport,
//! mocked sessions and an isolated Python process that emits fixed RGB bytes.
use kagnode_entropy::{
    crypto::RandomSource,
    engine::{Engine, Error, Mode, Sample},
    video::{
        self, DecoderFactory, DecoderSession, EncodedInput, FfmpegFactory, Operation, VideoConfig,
        VideoError, VideoSource, VideoTransport,
    },
    video_http::{checked_addresses, public_address, validate_response, validate_url},
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::watch;
use zeroize::Zeroizing;
fn config() -> VideoConfig {
    VideoConfig {
        url: "https://video.example.invalid/fixture.mp4".into(),
        decoder: PathBuf::from("/usr/bin/ffmpeg"),
        width: 1,
        height: 1,
        fps: 1,
        frame_timeout: Duration::from_millis(100),
        session_lifetime: Duration::from_secs(1),
        max_input_bytes: 1024,
        memory_mib: 128,
        retries: 1,
        retry_delay: Duration::from_millis(100),
    }
}
struct Fixed;
impl RandomSource for Fixed {
    fn fill(&self, out: &mut [u8]) -> Result<(), &'static str> {
        out.fill(6);
        Ok(())
    }
}
fn engine(cap: usize) -> Engine {
    Engine::new(Mode::OsBacked, cap, Arc::new(Fixed), None).unwrap()
}
fn counters() -> Zeroizing<Vec<u8>> {
    Zeroizing::new(b"fictional counters".to_vec())
}
enum Behavior {
    Pixels(Vec<u8>),
    FrameThenLimit(Vec<u8>),
    Failure(VideoError),
    Pending,
    Unreapable,
}
struct FakeFactory {
    behaviors: Mutex<VecDeque<Behavior>>,
    starts: Arc<AtomicUsize>,
    stops: Arc<AtomicUsize>,
}
impl FakeFactory {
    fn new(items: Vec<Behavior>) -> Arc<Self> {
        Arc::new(Self {
            behaviors: Mutex::new(items.into()),
            starts: Arc::new(AtomicUsize::new(0)),
            stops: Arc::new(AtomicUsize::new(0)),
        })
    }
}
struct FakeSession {
    behavior: Behavior,
    reads: usize,
    stops: Arc<AtomicUsize>,
}
impl DecoderSession for FakeSession {
    fn read_frame<'a>(&'a mut self, out: &'a mut [u8]) -> Operation<'a, ()> {
        Box::pin(async move {
            let first = self.reads == 0;
            self.reads += 1;
            match &self.behavior {
                Behavior::Pixels(bytes) => {
                    if bytes.len() != out.len() {
                        return Err(VideoError::TruncatedFrame);
                    }
                    out.copy_from_slice(bytes);
                    Ok(())
                }
                Behavior::FrameThenLimit(bytes) => {
                    if !first {
                        return Err(VideoError::InputLimit);
                    }
                    out.copy_from_slice(bytes);
                    Ok(())
                }
                Behavior::Failure(error) => Err(*error),
                Behavior::Pending | Behavior::Unreapable => std::future::pending().await,
            }
        })
    }
    fn stop(&mut self) -> Operation<'_, ()> {
        Box::pin(async move {
            self.stops.fetch_add(1, Ordering::SeqCst);
            if matches!(self.behavior, Behavior::Unreapable) {
                Err(VideoError::Reap)
            } else {
                Ok(())
            }
        })
    }
}
impl DecoderFactory for FakeFactory {
    fn start<'a>(&'a self, _: &'a VideoConfig) -> Operation<'a, Box<dyn DecoderSession>> {
        Box::pin(async move {
            self.starts.fetch_add(1, Ordering::SeqCst);
            let b = self
                .behaviors
                .lock()
                .unwrap()
                .pop_front()
                .ok_or(VideoError::Spawn)?;
            Ok(Box::new(FakeSession {
                behavior: b,
                reads: 0,
                stops: self.stops.clone(),
            }) as Box<dyn DecoderSession>)
        })
    }
}
#[test]
fn public_url_policy_rejects_local_credentials_and_protocols() {
    for value in [
        "file:///etc/passwd",
        "https://user:pass@example.org/",
        "http://127.0.0.1/a",
        "http://2130706433/a",
        "http://[::1]/a",
        "http://[::ffff:127.0.0.1]/a",
        "http://169.254.169.254/a",
        "https://localhost./a",
        "https://name.local/a",
        "https://example.org:25/a",
        "https://example.org/a#part",
    ] {
        assert!(validate_url(value).is_err(), "{value}");
    }
    assert!(validate_url("https://video.example.invalid/file.mp4?quality=small").is_ok());
}
#[test]
fn dns_filter_rejects_mixed_private_answers_and_transition_addresses() {
    for value in [
        "10.0.0.1",
        "100.64.0.1",
        "192.0.2.1",
        "198.18.0.1",
        "192.88.99.1",
        "224.0.0.1",
        "fd00::1",
        "fe80::1",
        "2001:db8::1",
        "2002:7f00:1::",
        "3fff::1",
        "3ffe::1",
    ] {
        assert!(!public_address(value.parse().unwrap()), "{value}");
    }
    assert!(public_address("8.8.8.8".parse().unwrap()));
    assert!(public_address("2606:4700::1111".parse().unwrap()));
    assert!(
        checked_addresses(vec![
            "8.8.8.8:0".parse().unwrap(),
            "10.0.0.1:0".parse().unwrap()
        ])
        .is_err()
    );
    assert!(checked_addresses(vec![]).is_err());
    assert!(checked_addresses(vec!["8.8.8.8:0".parse().unwrap(); 33]).is_err());
}
#[test]
fn response_type_status_and_length_are_checked() {
    assert!(validate_response(200, Some(20), "video/mp4", 100).is_ok());
    assert_eq!(
        validate_response(404, None, "video/mp4", 100),
        Err(VideoError::Network)
    );
    assert_eq!(
        validate_response(200, Some(101), "video/mp4", 100),
        Err(VideoError::InputLimit)
    );
    for media in [
        "text/html",
        "application/vnd.apple.mpegurl",
        "application/json",
        "",
    ] {
        assert_eq!(
            validate_response(200, None, media, 100),
            Err(VideoError::Configuration)
        );
    }
}
#[test]
fn decoder_arguments_never_include_url_shell_or_network_protocols() {
    let c = config();
    let args = c.decoder_args();
    assert!(args.windows(2).any(|v| v == ["-i", "pipe:0"]));
    assert!(
        args.windows(2)
            .any(|v| v == ["-protocol_whitelist", "pipe"])
    );
    assert!(args.windows(2).any(|v| v == ["-pix_fmt", "rgb24"]));
    assert!(
        !args
            .iter()
            .any(|v| v.contains("https:") || v == "-y" || v == "sh")
    );
    let command = video::decoder_command(&c).unwrap();
    assert_eq!(command.as_std().get_program(), "/usr/bin/ffmpeg");
    let env: Vec<_> = command.as_std().get_envs().collect();
    assert_eq!(env.len(), 2);
}
#[test]
fn invalid_geometry_and_limits_fail_before_factory() {
    let mut c = config();
    c.width = u32::MAX;
    assert!(c.validate().is_err());
    c = config();
    c.max_input_bytes = 257 * 1024 * 1024;
    assert!(c.validate().is_err());
    c = config();
    c.decoder = PathBuf::from("ffmpeg");
    assert!(c.validate().is_err());
    c = config();
    c.retries = 6;
    assert!(c.validate().is_err());
}
#[tokio::test]
async fn decoded_pixels_reach_exact_three_stage_pipeline() {
    let f = FakeFactory::new(vec![Behavior::Pixels(vec![3, 7, 9])]);
    let mut source = VideoSource::with_clock(config(), f.clone(), Arc::new(|| 42)).unwrap();
    let e = engine(1);
    let reference = engine(1);
    let (_tx, mut rx) = watch::channel(false);
    assert!(source.step(&e, counters(), &mut rx).await.unwrap());
    reference
        .ingest(&Sample {
            pixels: Zeroizing::new(vec![3, 7, 9]),
            timestamp_ns: 42,
            counters: counters(),
            geometry: Some([1, 1, 3]),
        })
        .unwrap();
    assert_eq!(
        e.consume("fixture", 1, "test").unwrap()[0].value,
        reference.consume("fixture", 1, "test").unwrap()[0].value
    );
    source.shutdown().await.unwrap();
    assert_eq!(f.stops.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn backpressure_prevents_fetch_spawn_and_consumption() {
    let f = FakeFactory::new(vec![Behavior::Pixels(vec![1, 2, 3])]);
    let mut source = VideoSource::new(config(), f.clone()).unwrap();
    let e = engine(1);
    let (_tx, mut rx) = watch::channel(false);
    assert!(source.step(&e, counters(), &mut rx).await.unwrap());
    assert!(!source.step(&e, counters(), &mut rx).await.unwrap());
    assert_eq!(f.starts.load(Ordering::SeqCst), 1);
    assert_eq!(source.status.lock().unwrap().frames, 1);
    let second = FakeFactory::new(vec![]);
    let mut blocked = VideoSource::new(config(), second.clone()).unwrap();
    assert!(!blocked.step(&e, counters(), &mut rx).await.unwrap());
    assert_eq!(second.starts.load(Ordering::SeqCst), 0);
    source.shutdown().await.unwrap();
}
#[tokio::test]
async fn partial_frame_is_discarded_and_retry_recovers_without_synthetic_fallback() {
    let f = FakeFactory::new(vec![
        Behavior::Pixels(vec![1, 2]),
        Behavior::Pixels(vec![3, 4, 5]),
    ]);
    let mut source = VideoSource::new(config(), f.clone()).unwrap();
    let e = engine(2);
    let (_tx, mut rx) = watch::channel(false);
    assert!(!source.step(&e, counters(), &mut rx).await.unwrap());
    assert_eq!(e.snapshot().unwrap().produced, 0);
    assert!(matches!(e.consume("a", 1, ""), Err(Error::Source)));
    assert!(source.step(&e, counters(), &mut rx).await.unwrap());
    assert!(!e.snapshot().unwrap().producer_fault);
    assert_eq!(f.starts.load(Ordering::SeqCst), 2);
    assert_eq!(f.stops.load(Ordering::SeqCst), 1);
    source.shutdown().await.unwrap();
}
#[tokio::test]
async fn retry_budget_limits_failures_and_refuses_additional_starts() {
    let f = FakeFactory::new(vec![
        Behavior::Failure(VideoError::EndOfStream),
        Behavior::Failure(VideoError::Network),
        Behavior::Pixels(vec![1, 2, 3]),
    ]);
    let mut source = VideoSource::new(config(), f.clone()).unwrap();
    let e = engine(1);
    let (_tx, mut rx) = watch::channel(false);
    assert!(!source.step(&e, counters(), &mut rx).await.unwrap());
    assert_eq!(
        source.step(&e, counters(), &mut rx).await,
        Err(VideoError::Network)
    );
    assert_eq!(
        source.step(&e, counters(), &mut rx).await,
        Err(VideoError::Stopped)
    );
    assert_eq!(f.starts.load(Ordering::SeqCst), 2);
    assert_eq!(source.status.lock().unwrap().state, "failed");
}
#[tokio::test]
async fn frame_timeout_stops_and_reaps_before_retry() {
    let f = FakeFactory::new(vec![Behavior::Pending, Behavior::Pixels(vec![1, 2, 3])]);
    let mut source = VideoSource::new(config(), f.clone()).unwrap();
    let e = engine(1);
    let (_tx, mut rx) = watch::channel(false);
    assert!(!source.step(&e, counters(), &mut rx).await.unwrap());
    assert_eq!(f.stops.load(Ordering::SeqCst), 1);
    assert!(source.step(&e, counters(), &mut rx).await.unwrap());
    source.shutdown().await.unwrap();
}
#[tokio::test]
async fn cancellation_interrupts_a_pending_frame_and_reaps() {
    let f = FakeFactory::new(vec![Behavior::Pending]);
    let mut source = VideoSource::new(config(), f.clone()).unwrap();
    let e = engine(1);
    let (tx, mut rx) = watch::channel(false);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        tx.send(true).unwrap();
    });
    assert_eq!(
        source.step(&e, counters(), &mut rx).await,
        Err(VideoError::Stopped)
    );
    assert_eq!(f.stops.load(Ordering::SeqCst), 1);
    assert_eq!(e.snapshot().unwrap().produced, 0);
}
#[tokio::test]
async fn cleanup_failure_prevents_another_process_start() {
    let f = FakeFactory::new(vec![Behavior::Unreapable, Behavior::Pixels(vec![1, 2, 3])]);
    let mut source = VideoSource::new(config(), f.clone()).unwrap();
    let e = engine(1);
    let (_tx, mut rx) = watch::channel(false);
    assert_eq!(
        source.step(&e, counters(), &mut rx).await,
        Err(VideoError::Reap)
    );
    assert_eq!(
        source.step(&e, counters(), &mut rx).await,
        Err(VideoError::Stopped)
    );
    assert_eq!(f.starts.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn session_frame_budget_renews_only_after_previous_session_reaped() {
    let f = FakeFactory::new(vec![
        Behavior::Pixels(vec![1, 2, 3]),
        Behavior::Pixels(vec![4, 5, 6]),
    ]);
    let mut source = VideoSource::new(config(), f.clone()).unwrap();
    let e = engine(2);
    let (_tx, mut rx) = watch::channel(false);
    assert!(source.step(&e, counters(), &mut rx).await.unwrap());
    assert!(source.step(&e, counters(), &mut rx).await.unwrap());
    assert_eq!(f.starts.load(Ordering::SeqCst), 2);
    assert_eq!(f.stops.load(Ordering::SeqCst), 1);
    assert_eq!(source.status.lock().unwrap().failures, 0);
    source.shutdown().await.unwrap();
}

struct FakeTransport {
    bytes: bytes::Bytes,
}
struct FakeInput {
    chunk: Option<bytes::Bytes>,
}
impl EncodedInput for FakeInput {
    fn next_chunk(&mut self) -> Operation<'_, Option<bytes::Bytes>> {
        Box::pin(async move { Ok(self.chunk.take()) })
    }
}
impl VideoTransport for FakeTransport {
    fn open<'a>(&'a self, _: &'a VideoConfig) -> Operation<'a, Box<dyn EncodedInput>> {
        Box::pin(async move {
            Ok(Box::new(FakeInput {
                chunk: Some(self.bytes.clone()),
            }) as Box<dyn EncodedInput>)
        })
    }
}
fn stub(root: &std::path::Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join("rgb-fixture");
    std::fs::write(&path,b"#!/usr/bin/python3\nimport os,sys,resource,time\nassert set(os.environ).issubset({'LANG','LC_ALL'})\nassert resource.getrlimit(resource.RLIMIT_FSIZE)==(0,0)\nassert resource.getrlimit(resource.RLIMIT_CORE)==(0,0)\nassert resource.getrlimit(resource.RLIMIT_AS)==(134217728,134217728)\nassert sys.argv[sys.argv.index('-protocol_whitelist')+1]=='pipe'\nassert sys.argv[sys.argv.index('-i')+1]=='pipe:0'\ndata=sys.stdin.buffer.read()\nif data!=b'fictional encoded media': sys.exit(42)\nsys.stdout.buffer.write(bytes([11,22,33]));sys.stdout.buffer.flush()\ntime.sleep(30)\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}
#[tokio::test]
async fn mock_transport_feeds_isolated_stub_and_owned_child_is_reaped() {
    let root = tempfile::tempdir().unwrap();
    let mut c = config();
    c.decoder = stub(root.path());
    c.frame_timeout = Duration::from_secs(2);
    let factory = FfmpegFactory {
        transport: Arc::new(FakeTransport {
            bytes: bytes::Bytes::from_static(b"fictional encoded media"),
        }),
    };
    let mut session = factory.start(&c).await.unwrap();
    let pid = session.process_id().unwrap();
    let mut frame = [0; 3];
    session.read_frame(&mut frame).await.unwrap();
    assert_eq!(frame, [11, 22, 33]);
    session.stop().await.unwrap();
    assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
    session.stop().await.unwrap();
}
#[tokio::test]
async fn oversized_encoded_input_is_rejected_before_decoder_receives_it() {
    let root = tempfile::tempdir().unwrap();
    let mut c = config();
    c.decoder = stub(root.path());
    c.frame_timeout = Duration::from_secs(2);
    let factory = FfmpegFactory {
        transport: Arc::new(FakeTransport {
            bytes: bytes::Bytes::from(vec![0; 1025]),
        }),
    };
    let mut session = factory.start(&c).await.unwrap();
    let result = session.read_frame(&mut [0; 3]).await;
    assert_eq!(result, Err(VideoError::InputLimit));
    session.stop().await.unwrap();
}

#[test]
fn redirects_cannot_downgrade_https_or_reach_private_targets() {
    use kagnode_entropy::video_http::validate_redirect;
    let origin = url::Url::parse("https://video.example.invalid/start").unwrap();
    assert!(
        validate_redirect(
            std::slice::from_ref(&origin),
            &url::Url::parse("http://example.invalid/media").unwrap()
        )
        .is_err()
    );
    assert!(
        validate_redirect(
            std::slice::from_ref(&origin),
            &url::Url::parse("https://127.0.0.1/media").unwrap()
        )
        .is_err()
    );
    assert!(validate_redirect(&[origin.clone(), origin.clone(), origin.clone()], &origin).is_err());
    assert!(
        validate_redirect(
            std::slice::from_ref(&origin),
            &url::Url::parse("https://cdn.example.invalid/media").unwrap()
        )
        .is_ok()
    );
}
#[tokio::test]
async fn counters_are_sampled_after_a_complete_frame_and_clock_only() {
    let f = FakeFactory::new(vec![
        Behavior::Pixels(vec![1, 2]),
        Behavior::Pixels(vec![3, 4, 5]),
    ]);
    let clock_called = Arc::new(AtomicUsize::new(0));
    let observed = clock_called.clone();
    let mut source = VideoSource::with_clock(
        config(),
        f,
        Arc::new(move || {
            observed.fetch_add(1, Ordering::SeqCst);
            42
        }),
    )
    .unwrap();
    let e = engine(1);
    let (_tx, mut rx) = watch::channel(false);
    assert!(
        !source
            .step_with_counters(
                &e,
                || panic!("partial frame must not sample counters"),
                &mut rx
            )
            .await
            .unwrap()
    );
    assert_eq!(clock_called.load(Ordering::SeqCst), 0);
    assert!(
        source
            .step_with_counters(
                &e,
                || {
                    assert_eq!(clock_called.load(Ordering::SeqCst), 1);
                    Ok(counters())
                },
                &mut rx
            )
            .await
            .unwrap()
    );
    assert!(
        !source
            .step_with_counters(&e, || panic!("full pool must not sample counters"), &mut rx)
            .await
            .unwrap()
    );
    source.shutdown().await.unwrap();
}

#[tokio::test]
async fn reached_transfer_budget_after_valid_frame_renews_without_spending_failure_retries() {
    let f = FakeFactory::new(vec![
        Behavior::FrameThenLimit(vec![1, 2, 3]),
        Behavior::Pixels(vec![4, 5, 6]),
    ]);
    let mut c = config();
    c.fps = 2;
    c.retries = 0;
    let mut source = VideoSource::new(c, f.clone()).unwrap();
    let e = engine(2);
    let (_tx, mut rx) = watch::channel(false);
    assert!(source.step(&e, counters(), &mut rx).await.unwrap());
    assert!(!source.step(&e, counters(), &mut rx).await.unwrap());
    assert!(source.step(&e, counters(), &mut rx).await.unwrap());
    assert_eq!(source.status.lock().unwrap().failures, 0);
    assert_eq!(f.starts.load(Ordering::SeqCst), 2);
    source.shutdown().await.unwrap();
}

#[test]
fn rfc9637_documentation_prefix_boundaries_are_rejected_everywhere() {
    use std::net::{IpAddr, SocketAddr};
    for address in [
        "3fff::",
        "3fff:800::1",
        "3fff:fff:ffff:ffff:ffff:ffff:ffff:ffff",
    ] {
        let ip: IpAddr = address.parse().unwrap();
        assert!(!public_address(ip));
        assert!(validate_url(&format!("https://[{address}]/media")).is_err());
        assert!(
            checked_addresses(vec![
                "2606:4700::1111"
                    .parse::<IpAddr>()
                    .map(|ip| SocketAddr::new(ip, 0))
                    .unwrap(),
                SocketAddr::new(ip, 0)
            ])
            .is_err()
        );
    }
    let after: IpAddr = "3fff:1000::".parse().unwrap();
    assert!(public_address(after));
    assert!(validate_url("https://[3fff:1000::]/media").is_ok());
}
#[tokio::test]
async fn engine_snapshot_failure_stops_active_decoder_for_library_callers() {
    let f = FakeFactory::new(vec![Behavior::Pixels(vec![1, 2, 3])]);
    let mut c = config();
    c.fps = 2;
    let mut source = VideoSource::new(c, f.clone()).unwrap();
    let e = engine(2);
    let (_tx, mut rx) = watch::channel(false);
    assert!(source.step(&e, counters(), &mut rx).await.unwrap());
    e.close();
    assert_eq!(
        source.step(&e, counters(), &mut rx).await,
        Err(VideoError::Engine)
    );
    assert_eq!(f.stops.load(Ordering::SeqCst), 1);
    assert_eq!(f.starts.load(Ordering::SeqCst), 1);
    assert_eq!(
        source.step(&e, counters(), &mut rx).await,
        Err(VideoError::Stopped)
    );
}
