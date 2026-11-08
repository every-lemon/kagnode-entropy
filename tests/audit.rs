use kagnode_entropy::{
    audit::{self, AuditConfig, AuditLog, Privacy},
    sources::SyntheticSource,
};
use std::{fs, time::Duration};
use zeroize::Zeroizing;
fn config() -> AuditConfig {
    AuditConfig {
        records_per_segment: 2,
        retained_segments: 2,
        privacy: Privacy::default(),
    }
}
#[test]
fn bounded_retention_and_independent_checkpoint() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("audit");
    let key = [7u8; 32];
    let mut audit = AuditLog::create(&dir, Zeroizing::new(key), config()).unwrap();
    for n in 1..=7 {
        audit.allocated(n, 1).unwrap();
    }
    let cp = audit.checkpoint();
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
    assert_eq!(cp.first_retained_index, 5);
    let verified = audit::verify(&dir, &key, cp.index, &cp.head).unwrap();
    assert_eq!(verified.first_retained_index, 5);
    assert!(audit::verify(&dir, &key, cp.index - 1, &cp.head).is_err());
}
#[test]
fn mutation_and_wrong_key_fail() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("audit");
    let mut log = AuditLog::create(&dir, Zeroizing::new([9; 32]), config()).unwrap();
    log.allocated(1, 1).unwrap();
    let cp = log.checkpoint();
    assert!(audit::verify(&dir, &[8; 32], cp.index, &cp.head).is_err());
    let file = fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path();
    let bytes = fs::read_to_string(&file)
        .unwrap()
        .replace("\"count\":1", "\"count\":2");
    fs::write(file, bytes).unwrap();
    assert!(audit::verify(&dir, &[9; 32], cp.index, &cp.head).is_err());
}
#[test]
fn truncation_detected_against_external_checkpoint() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("audit");
    let mut log = AuditLog::create(&dir, Zeroizing::new([9; 32]), config()).unwrap();
    log.allocated(1, 1).unwrap();
    log.allocated(2, 1).unwrap();
    let cp = log.checkpoint();
    let path = fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path();
    let text = fs::read_to_string(&path).unwrap();
    fs::write(path, format!("{}\n", text.lines().next().unwrap())).unwrap();
    assert!(audit::verify(&dir, &[9; 32], cp.index, &cp.head).is_err());
}
#[test]
fn default_privacy_never_records_frames_outputs_or_client_ids() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("audit");
    let mut log = AuditLog::create(&dir, Zeroizing::new([9; 32]), config()).unwrap();
    log.produced(
        1,
        &SyntheticSource::default().sample(),
        Duration::from_nanos(1234),
    )
    .unwrap();
    log.allocated(1, 1).unwrap();
    let text =
        fs::read_to_string(fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path()).unwrap();
    for field in [
        "duration_ns",
        "geometry",
        "frame_fingerprint",
        "network_totals",
        "pixels",
        "client",
        "output",
        "key",
    ] {
        assert!(!text.contains(field), "{field}");
    }
}
#[test]
fn detailed_metadata_is_opt_in_and_not_raw_pixels() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("audit");
    let mut cfg = config();
    cfg.privacy = Privacy {
        timing: true,
        network: true,
        frame_fingerprint: true,
    };
    let mut log = AuditLog::create(&dir, Zeroizing::new([9; 32]), cfg).unwrap();
    let sample = SyntheticSource::default().sample();
    log.produced(1, &sample, Duration::from_nanos(1234))
        .unwrap();
    let text =
        fs::read_to_string(fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path()).unwrap();
    assert!(text.contains("\"duration_ns\":1234"));
    assert!(text.contains("frame_fingerprint"));
    assert!(text.contains("geometry"));
    assert!(!text.contains("network_totals"));
}
#[test]
fn refuses_existing_directory_and_symlink_key() {
    let root = tempfile::tempdir().unwrap();
    assert!(AuditLog::create(root.path(), Zeroizing::new([9; 32]), config()).is_err());
    let path = root.path().join("test.key");
    audit::create_key(&path).unwrap();
    assert_eq!(audit::read_key(&path).unwrap().len(), 32);
    assert!(audit::create_key(&path).is_err());
    #[cfg(unix)]
    {
        let link = root.path().join("link.key");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(audit::read_key(&link).is_err());
    }
}
#[test]
fn audit_io_failure_is_sticky() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("audit");
    let mut log = AuditLog::create(&dir, Zeroizing::new([9; 32]), config()).unwrap();
    log.allocated(1, 1).unwrap();
    log.allocated(2, 1).unwrap();
    fs::remove_dir_all(&dir).unwrap();
    assert!(log.allocated(3, 1).is_err());
    fs::create_dir(&dir).unwrap();
    assert!(log.allocated(4, 1).is_err());
}

#[test]
fn allocation_audit_failure_burns_removed_chunk_and_blocks_future_output() {
    use kagnode_entropy::{
        crypto::RandomSource,
        engine::{Engine, Error, Mode},
    };
    struct Fixed;
    impl RandomSource for Fixed {
        fn fill(&self, out: &mut [u8]) -> Result<(), &'static str> {
            out.fill(6);
            Ok(())
        }
    }
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("audit");
    let mut cfg = config();
    cfg.records_per_segment = 1;
    let audit = AuditLog::create(&dir, Zeroizing::new([9; 32]), cfg).unwrap();
    let e = Engine::new(Mode::OsBacked, 1, std::sync::Arc::new(Fixed), Some(audit)).unwrap();
    e.ingest(&SyntheticSource::default().sample()).unwrap();
    fs::remove_dir_all(&dir).unwrap();
    assert!(matches!(
        e.consume("fictional-client", 1, ""),
        Err(Error::Audit)
    ));
    let state = e.snapshot().unwrap();
    assert_eq!(state.queued, 0);
    assert_eq!(state.consumed, 1);
    assert!(state.producer_fault);
    assert!(matches!(
        e.consume("fictional-client", 1, ""),
        Err(Error::Source)
    ));
}
