use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use clap::{Args, Parser, Subcommand};
#[cfg(not(target_os = "linux"))]
compile_error!("This source preview currently supports Linux only.");

use kagnode_entropy::{
    audit::{self, AuditConfig, AuditLog, Privacy},
    crypto::{OsRandom, RandomSource},
    defense,
    engine::{Engine, Error, Mode},
    server::{self, AppState, Registry},
    sources::{self, SyntheticSource},
};
use std::{path::PathBuf, sync::Arc, time::Duration};
use zeroize::Zeroizing;

#[derive(Parser)]
#[command(
    version,
    about = "KagNode Entropy — Rust source preview; no assessed supplemental entropy"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Sample {
        #[command(flatten)]
        common: Common,
        #[arg(long,default_value_t=1,value_parser=clap::value_parser!(u8).range(1..=16))]
        count: u8,
    },
    Serve {
        #[command(flatten)]
        common: Common,
        #[arg(long,default_value_t=8098,value_parser=clap::value_parser!(u16).range(1024..))]
        port: u16,
        #[arg(long,default_value_t=128,value_parser=clap::value_parser!(u16).range(1..=4096))]
        capacity: u16,
        #[arg(long,default_value_t=100,value_parser=clap::value_parser!(u64).range(20..=60000))]
        interval_ms: u64,
        #[arg(long)]
        stop_if_traced: bool,
    },
    AuditKey {
        #[arg(long)]
        path: PathBuf,
    },
    AuditVerify {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        key_file: PathBuf,
        #[arg(long)]
        expected_index: u64,
        #[arg(long)]
        expected_head: String,
    },
}
#[derive(Args)]
struct Common {
    #[arg(long,value_enum,default_value_t=Mode::OsBacked)]
    mode: Mode,
    #[arg(long)]
    acknowledge_experimental: bool,
    #[arg(long)]
    linux_counters: bool,
    #[arg(long, requires = "audit_key_file")]
    audit_directory: Option<PathBuf>,
    #[arg(long, requires = "audit_directory")]
    audit_key_file: Option<PathBuf>,
    #[arg(long,default_value_t=1024,value_parser=clap::value_parser!(u16).range(1..=10000))]
    audit_segment_records: u16,
    #[arg(long,default_value_t=4,value_parser=clap::value_parser!(u8).range(2..=32))]
    audit_retained_segments: u8,
    #[arg(long, requires = "audit_directory")]
    audit_timing: bool,
    #[arg(long, requires = "audit_directory")]
    audit_network: bool,
    #[arg(long, requires = "audit_directory")]
    audit_frame_fingerprint: bool,
}
fn make_engine(
    common: &Common,
    capacity: usize,
) -> Result<Arc<Engine>, Box<dyn std::error::Error>> {
    if common.mode == Mode::Experimental && !common.acknowledge_experimental {
        return Err("Experimental output is unassessed; pass --acknowledge-experimental".into());
    }
    let log = if let (Some(dir), Some(key)) = (&common.audit_directory, &common.audit_key_file) {
        Some(AuditLog::create(
            dir,
            audit::read_key(key)?,
            AuditConfig {
                records_per_segment: common.audit_segment_records as usize,
                retained_segments: common.audit_retained_segments as usize,
                privacy: Privacy {
                    timing: common.audit_timing,
                    network: common.audit_network,
                    frame_fingerprint: common.audit_frame_fingerprint,
                },
            },
        )?)
    } else {
        None
    };
    Ok(Arc::new(Engine::new(
        common.mode,
        capacity,
        Arc::new(OsRandom),
        log,
    )?))
}
fn ingest(
    engine: &Engine,
    source: &mut SyntheticSource,
    linux_counters: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if engine.snapshot()?.queued >= engine.capacity {
        return Ok(());
    }
    let mut sample = source.sample();
    if linux_counters {
        sample.counters = sources::linux_network_counters()?;
    }
    match engine.ingest(&sample) {
        Ok(_) | Err(Error::Full) => Ok(()),
        Err(e) => Err(e.into()),
    }
}
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().command {
        Command::AuditKey { path } => {
            audit::create_key(&path)?;
            println!("Created a private audit key. Keep it separate from any retained checkpoint.");
        }
        Command::AuditVerify {
            directory,
            key_file,
            expected_index,
            expected_head,
        } => {
            let key = audit::read_key(&key_file)?;
            println!(
                "{}",
                serde_json::to_string(&audit::verify(
                    &directory,
                    key.as_ref(),
                    expected_index,
                    &expected_head
                )?)?
            );
        }
        Command::Sample { common, count } => {
            let engine = make_engine(&common, count as usize)?;
            let mut source = SyntheticSource::default();
            for _ in 0..count {
                ingest(&engine, &mut source, common.linux_counters)?;
            }
            let blocks = engine.consume("local-cli", count as usize, "")?;
            println!(
                "{}",
                serde_json::json!({"mode":common.mode,"source":"synthetic pixels","blocks":blocks.iter().map(|d|STANDARD.encode(d.value.as_ref())).collect::<Vec<_>>()})
            );
            print_checkpoint(&engine);
            engine.close();
        }
        Command::Serve {
            common,
            port,
            capacity,
            interval_ms,
            stop_if_traced,
        } => {
            let engine = make_engine(&common, capacity as usize)?;
            if stop_if_traced
                && !defense::apply_tracer_observation(&engine, defense::own_process_traced())
            {
                engine.close();
                return Err("Tracer policy stopped this instance".into());
            }
            let mut tokens = Vec::new();
            for id in ["preview-alice", "preview-bob"] {
                let mut bytes = Zeroizing::new([0u8; 32]);
                OsRandom.fill(bytes.as_mut())?;
                tokens.push((URL_SAFE_NO_PAD.encode(bytes.as_ref()), id.to_owned()));
            }
            let listener =
                tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
            println!("KagNode Entropy · http://127.0.0.1:{port} · local source preview");
            println!("Disposable local tokens (shown once; restart replaces them):");
            for (token, id) in &tokens {
                println!("  {id}: {token}");
            }
            let state = AppState {
                engine: engine.clone(),
                registry: Arc::new(Registry::new(tokens, 16.0, 32)?),
                port,
                linux_counters: common.linux_counters,
            };
            let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
            let mut runner = tokio::spawn(server::serve(listener, state, stop_rx));
            let mut source = SyntheticSource::default();
            let mut ticker = tokio::time::interval(Duration::from_millis(interval_ms));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut defense_tick = tokio::time::interval(Duration::from_millis(250));
            #[cfg(unix)]
            let mut terminate =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            let mut stopped_for_failure = false;
            loop {
                tokio::select! {
                    _=ticker.tick()=>{if ingest(&engine,&mut source,common.linux_counters).is_err(){engine.source_failed(); stopped_for_failure = true; eprintln!("Producer stopped after a source or audit failure.");break;}},
                    _=defense_tick.tick(),if stop_if_traced=>{if !defense::apply_tracer_observation(&engine, defense::own_process_traced()){stopped_for_failure = true; eprintln!("Own-process tracer policy stopped this instance.");break;}},
                    _=tokio::signal::ctrl_c()=>{break;},
                    _=terminate.recv()=>{break;},
                    result=&mut runner=>{engine.close(); result??; return Err("Preview listener stopped unexpectedly".into());},
                }
            }
            print_checkpoint(&engine);
            engine.close();
            let _ = stop_tx.send(true);
            runner.await??;
            if stopped_for_failure {
                return Err("Instance stopped by source, audit or tracer policy".into());
            }
        }
    }
    Ok(())
}
fn print_checkpoint(engine: &Engine) {
    if let Some(checkpoint) = engine.audit_checkpoint() {
        eprintln!(
            "Audit checkpoint (retain independently): {}",
            serde_json::to_string(&checkpoint).unwrap()
        );
    }
}
