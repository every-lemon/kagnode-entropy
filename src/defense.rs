//! Opt-in own-process tracer check, not an anti-root or instantaneous defense.
use std::{
    fs::File,
    io::{self, Read},
};
pub fn parse_tracer(status: &str) -> io::Result<bool> {
    let mut found = None;
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("TracerPid:") {
            if found.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Duplicate tracer field",
                ));
            }
            found = Some(
                value.trim().parse::<u32>().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "Malformed tracer field")
                })? != 0,
            );
        }
    }
    found.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Tracer field unavailable"))
}
pub fn own_process_traced() -> io::Result<bool> {
    let mut status = String::new();
    File::open("/proc/self/status")?
        .take(65537)
        .read_to_string(&mut status)?;
    if status.len() > 65536 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Process status exceeds bounds",
        ));
    }
    parse_tracer(&status)
}

/// Apply one already-obtained observation. Read failures fail closed only when
/// the operator enabled this policy. A positive result is not proof of attack.
pub fn apply_tracer_observation(
    engine: &crate::engine::Engine,
    observation: io::Result<bool>,
) -> bool {
    if matches!(observation, Ok(false)) {
        true
    } else {
        engine.close();
        false
    }
}
