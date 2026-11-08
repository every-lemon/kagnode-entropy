use std::process::Command;
#[test]
fn invalid_source_configuration_does_not_create_an_audit_session() {
    let directory = tempfile::tempdir().unwrap();
    let key = directory.path().join("fixture.key");
    let audit = directory.path().join("uncreated-audit");
    let binary = env!("CARGO_BIN_EXE_kagnode-entropy");
    let made = Command::new(binary)
        .args(["audit-key", "--path"])
        .arg(&key)
        .output()
        .unwrap();
    assert!(made.status.success());
    let rejected = Command::new(binary)
        .args([
            "sample",
            "--video-url",
            "file:///fictional-not-read",
            "--audit-key-file",
        ])
        .arg(&key)
        .arg("--audit-directory")
        .arg(&audit)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty());
    assert!(!audit.exists());
}

#[test]
fn sample_cancellation_handles_sigterm_during_a_source_retry() {
    use std::{
        thread,
        time::{Duration, Instant},
    };
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("absent-fixture-decoder");
    let mut child = Command::new(env!("CARGO_BIN_EXE_kagnode-entropy"))
        .args([
            "sample",
            "--video-url",
            "https://video.example.invalid/fictional.mp4",
            "--video-decoder",
        ])
        .arg(&missing)
        .args(["--video-retries", "5"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // Observe only this owned fixture's signal registration, never other users'
    // processes. The absent decoder is rejected before the HTTP transport opens.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut ready = false;
    while Instant::now() < deadline {
        let text =
            std::fs::read_to_string(format!("/proc/{}/status", child.id())).unwrap_or_default();
        ready = text
            .lines()
            .find_map(|line| line.strip_prefix("SigCgt:"))
            .and_then(|v| u64::from_str_radix(v.trim(), 16).ok())
            .is_some_and(|bits| bits & (1 << (libc::SIGTERM - 1)) != 0);
        if ready {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    if !ready {
        let _ = child.kill();
        let _ = child.wait();
        panic!("Sample did not register graceful termination");
    }
    // SAFETY: this is the live, unreaped child created immediately above.
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGTERM) }, 0);
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
}
