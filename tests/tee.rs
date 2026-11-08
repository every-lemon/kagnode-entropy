use kagnode_entropy::tee::{MockEvidence, Policy};
fn sgx() -> Policy {
    Policy::Sgx {
        mrenclave: "1".repeat(64),
        mrsigner: "2".repeat(64),
        product_id: 7,
        minimum_svn: 2,
        max_age_seconds: 60,
    }
}
fn evidence() -> MockEvidence {
    MockEvidence {
        measurements: sgx(),
        debug: false,
        nonce: "3".repeat(64),
        channel_key_sha256: "4".repeat(64),
        issued_at: 100,
    }
}
#[test]
fn matching_mock_never_authenticates_hardware_or_releases_secrets() {
    let result = sgx().inspect_mock(&evidence(), &"3".repeat(64), &"4".repeat(64), 110);
    assert!(result.fixture_policy_matches);
    assert!(!result.hardware_verified);
    assert!(!result.may_release_secrets);
}
#[test]
fn replay_stale_future_nonce_and_channel_mismatch_fail() {
    let p = sgx();
    for (nonce, key, now) in [
        ("3".repeat(64), "4".repeat(64), 161),
        ("3".repeat(64), "4".repeat(64), 99),
        ("5".repeat(64), "4".repeat(64), 110),
        ("3".repeat(64), "5".repeat(64), 110),
    ] {
        assert!(
            !p.inspect_mock(&evidence(), &nonce, &key, now)
                .fixture_policy_matches
        );
    }
}
#[test]
fn debug_and_svn_rollback_rejected() {
    let mut e = evidence();
    e.debug = true;
    assert!(
        !sgx()
            .inspect_mock(&e, &"3".repeat(64), &"4".repeat(64), 110)
            .fixture_policy_matches
    );
    e.debug = false;
    if let Policy::Sgx { minimum_svn, .. } = &mut e.measurements {
        *minimum_svn = 1;
    }
    assert!(
        !sgx()
            .inspect_mock(&e, &"3".repeat(64), &"4".repeat(64), 110)
            .fixture_policy_matches
    );
}
#[test]
fn platform_profiles_and_zero_measurements_do_not_mix() {
    let nitro = Policy::Nitro {
        pcr0: "1".repeat(96),
        pcr1: "2".repeat(96),
        pcr2: "3".repeat(96),
        max_age_seconds: 60,
    };
    assert!(nitro.validate().is_ok());
    assert!(
        !nitro
            .inspect_mock(&evidence(), &"3".repeat(64), &"4".repeat(64), 110)
            .fixture_policy_matches
    );
    let zero = Policy::Nitro {
        pcr0: "0".repeat(96),
        pcr1: "2".repeat(96),
        pcr2: "3".repeat(96),
        max_age_seconds: 60,
    };
    assert!(zero.validate().is_err());
}
#[test]
fn unknown_policy_fields_rejected() {
    let text = r#"{"profile":"nitro","pcr0":"x","pcr1":"x","pcr2":"x","max_age_seconds":60,"trust_all":true}"#;
    assert!(serde_json::from_str::<Policy>(text).is_err());
}
