use kagnode_entropy::{
    crypto::{self, RandomSource},
    defense::parse_tracer,
    engine::{Engine, Error, Mode, Sample},
    sources::{SyntheticSource, network_totals},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use zeroize::Zeroizing;
struct Fixed {
    calls: AtomicUsize,
    fail: AtomicBool,
}
impl Fixed {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
        })
    }
}
impl RandomSource for Fixed {
    fn fill(&self, out: &mut [u8]) -> Result<(), &'static str> {
        if self.fail.load(Ordering::SeqCst) {
            return Err("mock unavailable");
        }
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        for (i, b) in out.iter_mut().enumerate() {
            *b = (n + i) as u8;
        }
        Ok(())
    }
}
fn sample() -> Sample {
    Sample {
        pixels: Zeroizing::new(vec![1, 2, 3]),
        timestamp_ns: 42,
        counters: Zeroizing::new(b"fictional counters".to_vec()),
        geometry: Some([1, 1, 3]),
    }
}
fn engine(capacity: usize) -> Engine {
    Engine::new(Mode::OsBacked, capacity, Fixed::new(), None).unwrap()
}
#[test]
fn hmac_rfc4231_case1() {
    let result = crypto::hmac512(&[0x0b; 20], b"Hi There");
    assert_eq!(hex::encode(result.as_ref()),"87aa7cdea5ef619d4ff0b4241a1d6cb02379f4e2ce4ec2787ad0b30545e17cde daa833b7d6b8a702038b274eaea3f4e4be9d914eeb61f1702e696c203a126854".replace(' ',""));
}
#[test]
fn hkdf_rfc5869_case1() {
    let hk = hkdf::Hkdf::<sha2::Sha256>::new(
        Some(&hex::decode("000102030405060708090a0b0c").unwrap()),
        &[0x0b; 22],
    );
    let mut out = [0u8; 42];
    hk.expand(&hex::decode("f0f1f2f3f4f5f6f7f8f9").unwrap(), &mut out)
        .unwrap();
    assert_eq!(
        hex::encode(out),
        "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
    );
}
#[test]
fn stage_order_and_unambiguous_fields() {
    let s1 = crypto::hmac512(&[2; 64], b"pixels");
    let s2 = crypto::hmac512(s1.as_ref(), &42u128.to_be_bytes());
    let s3 = crypto::hmac512(s2.as_ref(), b"counts");
    assert_eq!(s3, crypto::blueprint(&[2; 64], b"pixels", 42, b"counts"));
    assert_ne!(crypto::frame(&[b"ab", b"c"]), crypto::frame(&[b"a", b"bc"]));
}
#[test]
fn fresh_os_input_and_backpressure() {
    let random = Fixed::new();
    let e = Engine::new(Mode::OsBacked, 1, random.clone(), None).unwrap();
    assert_eq!(random.calls.load(Ordering::SeqCst), 2);
    e.ingest(&sample()).unwrap();
    assert_eq!(random.calls.load(Ordering::SeqCst), 3);
    assert_eq!(e.ingest(&sample()), Err(Error::Full));
    assert_eq!(random.calls.load(Ordering::SeqCst), 3);
}
#[test]
fn experimental_repeatability_is_not_entropy_evidence() {
    let a = Engine::new(Mode::Experimental, 1, Fixed::new(), None).unwrap();
    let b = Engine::new(Mode::Experimental, 1, Fixed::new(), None).unwrap();
    for e in [&a, &b] {
        e.ingest(&sample()).unwrap();
    }
    assert_eq!(
        a.consume("test", 1, "").unwrap()[0].value,
        b.consume("test", 1, "").unwrap()[0].value
    );
}
#[test]
fn identity_context_and_next_generation_separate_fixed_fixture() {
    let run = |id: &str, context: &str| {
        let e = Engine::new(Mode::Experimental, 2, Fixed::new(), None).unwrap();
        e.ingest(&sample()).unwrap();
        e.ingest(&sample()).unwrap();
        e.consume(id, 2, context).unwrap()
    };
    let a = run("alice", "");
    let b = run("bob", "");
    let c = run("alice", "other");
    assert_ne!(a[0].value, a[1].value);
    assert_ne!(a[0].value, b[0].value);
    assert_ne!(a[0].value, c[0].value);
}
#[test]
fn insufficient_batch_is_atomic() {
    let e = engine(2);
    e.ingest(&sample()).unwrap();
    assert!(matches!(e.consume("alice", 2, ""), Err(Error::Empty)));
    assert_eq!(e.snapshot().unwrap().queued, 1);
    assert_eq!(e.consume("alice", 1, "").unwrap()[0].sequence, 1);
}
#[test]
fn concurrent_consumers_allocate_each_sequence_once() {
    let e = Arc::new(engine(128));
    for _ in 0..128 {
        e.ingest(&sample()).unwrap();
    }
    let jobs: Vec<_> = (0..8)
        .map(|_| {
            let e = e.clone();
            std::thread::spawn(move || {
                (0..20)
                    .filter_map(|_| e.consume("worker", 1, "").ok().map(|v| v[0].sequence))
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let mut receipts: Vec<_> = jobs.into_iter().flat_map(|j| j.join().unwrap()).collect();
    receipts.sort();
    assert_eq!(receipts, (1..=128).collect::<Vec<_>>());
    assert_eq!(e.snapshot().unwrap().consumed, 128);
}
#[test]
fn concurrent_producers_remain_bounded() {
    let e = Arc::new(engine(16));
    let jobs: Vec<_> = (0..8)
        .map(|_| {
            let e = e.clone();
            std::thread::spawn(move || {
                for _ in 0..16 {
                    let _ = e.ingest(&sample());
                }
            })
        })
        .collect();
    for job in jobs {
        job.join().unwrap();
    }
    assert_eq!(e.snapshot().unwrap().queued, 16);
    assert_eq!(e.snapshot().unwrap().produced, 16);
}
#[test]
fn rng_failure_blocks_allocations_without_advancing() {
    let random = Fixed::new();
    let e = Engine::new(Mode::OsBacked, 3, random.clone(), None).unwrap();
    e.ingest(&sample()).unwrap();
    random.fail.store(true, Ordering::SeqCst);
    assert_eq!(e.ingest(&sample()), Err(Error::Source));
    assert!(matches!(e.consume("a", 1, ""), Err(Error::Source)));
    assert_eq!(e.snapshot().unwrap().produced, 1);
    random.fail.store(false, Ordering::SeqCst);
    e.ingest(&sample()).unwrap();
    assert_eq!(e.consume("a", 1, "").unwrap()[0].sequence, 1);
}
#[test]
fn startup_failure_no_fallback() {
    let random = Fixed::new();
    random.fail.store(true, Ordering::SeqCst);
    assert!(matches!(
        Engine::new(Mode::OsBacked, 1, random, None),
        Err(Error::Source)
    ));
}
#[test]
fn source_failure_and_close_fail_closed() {
    let e = engine(1);
    e.ingest(&sample()).unwrap();
    e.source_failed();
    assert!(matches!(e.consume("a", 1, ""), Err(Error::Source)));
    e.close();
    e.close();
    assert!(matches!(e.snapshot(), Err(Error::Closed)));
}
#[test]
fn bounds_and_geometry() {
    let e = engine(1);
    let mut s = sample();
    s.geometry = Some([2, 1, 3]);
    assert_eq!(e.ingest(&s), Err(Error::Invalid));
    s.geometry = None;
    s.pixels = Zeroizing::new(vec![0; 1024 * 1024 + 1]);
    assert_eq!(e.ingest(&s), Err(Error::Invalid));
    assert!(matches!(e.consume("a", 0, ""), Err(Error::Invalid)));
    assert!(matches!(e.consume("a", 17, ""), Err(Error::Invalid)));
    assert!(matches!(
        e.consume("a", 1, &"x".repeat(129)),
        Err(Error::Invalid)
    ));
}
#[test]
fn synthetic_source_and_network_parsing_are_explicit() {
    let mut s = SyntheticSource::default();
    s.sample().validate().unwrap();
    assert_ne!(s.sample().pixels, s.sample().pixels);
    let fake=b"Inter- | Receive | Transmit\n fake0: 12 0 0 0 0 0 0 0 34 0 0 0 0 0 0 0\n fake1: 10 0 0 0 0 0 0 0 20 0 0 0 0 0 0 0";
    assert_eq!(network_totals(fake), Some([22, 54]));
    assert_eq!(network_totals(b"fake: invalid"), None);
}
#[test]
fn tracer_parser_handles_valid_invalid_and_duplicate() {
    assert!(!parse_tracer("Name:\ttest\nTracerPid:\t0\n").unwrap());
    assert!(parse_tracer("TracerPid:\t123\n").unwrap());
    assert!(parse_tracer("TracerPid: x").is_err());
    assert!(parse_tracer("Name: missing").is_err());
    assert!(parse_tracer("TracerPid: 0\nTracerPid: 9").is_err());
}

#[test]
fn opted_in_tracer_observation_stops_only_the_owned_engine() {
    let e = engine(1);
    let independent = engine(1);
    e.ingest(&sample()).unwrap();
    assert!(kagnode_entropy::defense::apply_tracer_observation(
        &e,
        Ok(false)
    ));
    assert!(!kagnode_entropy::defense::apply_tracer_observation(
        &e,
        Ok(true)
    ));
    assert!(matches!(e.snapshot(), Err(Error::Closed)));
    assert!(independent.snapshot().is_ok());
    assert!(!kagnode_entropy::defense::apply_tracer_observation(
        &independent,
        Err(std::io::Error::other("mock status unavailable"))
    ));
    assert!(matches!(independent.snapshot(), Err(Error::Closed)));
}

#[test]
fn production_output_is_exact_os_bytes_not_a_custom_derivation() {
    let e = engine(1);
    e.ingest(&sample()).unwrap();
    let output = e.consume("alice", 1, "public-context").unwrap();
    let expected: Vec<u8> = (0..64).map(|i| (i + 2) as u8).collect();
    assert_eq!(output[0].value.as_ref(), expected.as_slice());
}
