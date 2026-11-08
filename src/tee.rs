//! Design-time policy fixtures only. There is NO SGX quote verifier, Nitro COSE
//! verifier or hardware-backed secret-release path in this source preview.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "profile", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Policy {
    Sgx {
        mrenclave: String,
        mrsigner: String,
        product_id: u16,
        minimum_svn: u16,
        max_age_seconds: u64,
    },
    Nitro {
        pcr0: String,
        pcr1: String,
        pcr2: String,
        max_age_seconds: u64,
    },
}
#[derive(Clone, Debug)]
pub struct MockEvidence {
    pub measurements: Policy,
    pub debug: bool,
    pub nonce: String,
    pub channel_key_sha256: String,
    pub issued_at: u64,
}
#[derive(Debug, Serialize)]
pub struct SimulationResult {
    pub fixture_policy_matches: bool,
    pub hardware_verified: bool,
    pub may_release_secrets: bool,
}
fn digest(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        && value.bytes().any(|b| b != b'0')
}
impl Policy {
    pub fn validate(&self) -> Result<(), &'static str> {
        let (valid, age) = match self {
            Self::Sgx {
                mrenclave,
                mrsigner,
                max_age_seconds,
                ..
            } => (
                digest(mrenclave, 64) && digest(mrsigner, 64),
                *max_age_seconds,
            ),
            Self::Nitro {
                pcr0,
                pcr1,
                pcr2,
                max_age_seconds,
            } => (
                digest(pcr0, 96) && digest(pcr1, 96) && digest(pcr2, 96),
                *max_age_seconds,
            ),
        };
        if !valid || !(1..=300).contains(&age) {
            return Err(
                "Use explicit nonzero measurement pins and a 1–300 second freshness window",
            );
        }
        Ok(())
    }
    /// Deliberately cannot produce an authenticated result. Inputs are untrusted
    /// synthetic fields, even if all fixture checks match.
    pub fn inspect_mock(
        &self,
        evidence: &MockEvidence,
        expected_nonce: &str,
        expected_channel_key: &str,
        now: u64,
    ) -> SimulationResult {
        let max_age = match self {
            Self::Sgx {
                max_age_seconds, ..
            }
            | Self::Nitro {
                max_age_seconds, ..
            } => *max_age_seconds,
        };
        let pins = match (self, &evidence.measurements) {
            (
                Self::Sgx {
                    mrenclave: a,
                    mrsigner: b,
                    product_id: c,
                    minimum_svn: d,
                    ..
                },
                Self::Sgx {
                    mrenclave: e,
                    mrsigner: f,
                    product_id: g,
                    minimum_svn: h,
                    ..
                },
            ) => a == e && b == f && c == g && h >= d,
            (
                Self::Nitro {
                    pcr0: a,
                    pcr1: b,
                    pcr2: c,
                    ..
                },
                Self::Nitro {
                    pcr0: d,
                    pcr1: e,
                    pcr2: f,
                    ..
                },
            ) => a == d && b == e && c == f,
            _ => false,
        };
        let matches = self.validate().is_ok()
            && evidence.measurements.validate().is_ok()
            && pins
            && !evidence.debug
            && digest(expected_nonce, 64)
            && digest(expected_channel_key, 64)
            && evidence.nonce == expected_nonce
            && evidence.channel_key_sha256 == expected_channel_key
            && now >= evidence.issued_at
            && now - evidence.issued_at <= max_age;
        SimulationResult {
            fixture_policy_matches: matches,
            hardware_verified: false,
            may_release_secrets: false,
        }
    }
}
