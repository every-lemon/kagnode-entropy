use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha512;
use zeroize::Zeroizing;

pub const DOMAIN: &[u8] = b"KagNode Entropy/source-preview/v1/";
pub type Secret = Zeroizing<[u8; 64]>;

pub fn hmac512(key: &[u8], data: &[u8]) -> Secret {
    let mut mac = Hmac::<Sha512>::new_from_slice(key).expect("HMAC accepts any key size");
    mac.update(data);
    Zeroizing::new(mac.finalize().into_bytes().into())
}

/// Length-prefixed fields avoid ambiguous concatenation. Units are UTF-8 bytes.
pub fn frame(fields: &[&[u8]]) -> Zeroizing<Vec<u8>> {
    let mut result = Zeroizing::new(Vec::new());
    for field in fields {
        result.extend_from_slice(&(field.len() as u64).to_be_bytes());
        result.extend_from_slice(field);
    }
    result
}

/// Preserve the proposed three stages, but never publish S3 or use it as next key.
pub fn blueprint(key: &[u8], pixels: &[u8], time: u128, counters: &[u8]) -> Secret {
    let s1 = hmac512(key, pixels);
    let s2 = hmac512(s1.as_ref(), &time.to_be_bytes());
    hmac512(s2.as_ref(), counters)
}

pub trait RandomSource: Send + Sync {
    fn fill(&self, output: &mut [u8]) -> Result<(), &'static str>;
}
pub struct OsRandom;
impl RandomSource for OsRandom {
    fn fill(&self, output: &mut [u8]) -> Result<(), &'static str> {
        getrandom::fill(output).map_err(|_| "OS random source unavailable")
    }
}
