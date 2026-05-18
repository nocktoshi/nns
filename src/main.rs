//! nns hull binary.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use nns::{api, state::AppState};
use nockapp::kernel::boot;
use nockapp::NockApp;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if nns::handle_early_cli() {
        return Ok(());
    }

    nns::apply_nns_config();

    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let mut cli = boot::default_boot_cli(false);
    // Match integration tests: prover hot state + `%scan-block` Tip5 paths
    // use more Nock stack than the default CLI `Normal` size.
    cli.stack_size = nns::boot_stack_size();
    boot::init_default_tracing(&cli);

    // --- Load settlement + NNS config from nns.toml ---
    let toml_path = std::env::var("NNS_CONFIG").unwrap_or_else(|_| "nns.toml".into());
    let toml_cfg = load_toml(&PathBuf::from(&toml_path));
    let settlement_toml = toml_cfg.settlement_toml();
    let settlement = vesl_core::SettlementConfig::resolve(
        None,  // cli_mode
        None,  // cli_chain_endpoint
        false, // cli_submit
        None,  // cli_tx_fee
        None,  // cli_coinbase_timelock_min
        None,  // cli_accept_timeout
        None,  // cli_seed_phrase
        &settlement_toml,
        None, // default_signing_key (unused for local)
    );

    println!("=== nns ===");
    println!("  settlement mode: {}", settlement.mode);
    println!(
        "  nns genesis height (protocol): {}",
        nns::chain::NNS_GENESIS_HEIGHT
    );

    // --- Boot the kernel ---
    let kernel_path = std::env::var("NNS_KERNEL_JAM").unwrap_or_else(|_| "nns.jam".into());
    let kernel = fs::read(&kernel_path)
        .map_err(|e| format!("failed to read kernel jam {kernel_path}: {e}"))?;

    // All durable hull + kernel state lives under a single
    // `.nns-data/` directory (relative to $NNS_DATA_DIR). We pass
    // the env-configured parent as `data_dir` and `".nns-data"` as
    // the app name to `boot::setup`, which internally joins them
    // to produce `$NNS_DATA_DIR/.nns-data/checkpoints/` and
    // `$NNS_DATA_DIR/.nns-data/pma/`. The mirror JSON sits
    // alongside them in the same `.nns-data` dir so everything
    // the hull writes at runtime is contained in one folder,
    // separate from the source tree.
    let data_parent = PathBuf::from(std::env::var("NNS_DATA_DIR").unwrap_or_else(|_| ".".into()));
    let state_dir = data_parent.join(".nns-data");
    fs::create_dir_all(&state_dir)?;

    // Install the STARK prover hot state so `%prove-arbitrary` /
    // `%prove-claim-in-stark` can produce real STARK artifacts. No-op when
    // kernel never calls `prove-computation`, so it is safe to always
    // install; pokes that only touch %claim / %set-primary pay nothing
    // for the extra jets beyond module load time.
    let prover_hot_state = zkvm_jetpack::hot::produce_prover_hot_state();

    let app: NockApp = boot::setup(
        &kernel,
        cli,
        prover_hot_state.as_slice(),
        ".nns-data",
        Some(data_parent.clone()),
    )
    .await?;

    println!("  kernel booted ({} bytes)", kernel.len());
    println!("  state dir: {}", state_dir.display());

    let state = Arc::new(AppState::new(app, state_dir, settlement));
    let _follower = nns::chain_follower::spawn(state.clone());

    // --- Start HTTP server ---
    let port: u16 = std::env::var("API_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3000);
    let bind: String = std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1".into());

    // We don't drive `NockApp::run()` (pokes happen directly from
    // axum handlers via the shared mutex), so nockapp's built-in
    // periodic save tick and save-on-exit paths never fire. We
    // compensate in two places:
    //
    //   1. HTTP handlers that poke call `AppState::persist_all` inline.
    //      The chain follower uses `maybe_persist_after_follower_scan`
    //      instead (batched full checkpoints — see `NNS_FOLLOWER_PERSIST_EVERY`).
    //   2. Here — race `api::serve` against SIGINT/SIGTERM and flush
    //      once more on shutdown so any follower batches since the last
    //      checkpoint are written before exit.
    //
    // Errors from the final flush are logged but swallowed: the
    // signal already committed us to exiting, and any prior
    // successful poke was already persisted by path (1).
    let serve_result = tokio::select! {
        r = api::serve(state.clone(), port, &bind) => r,
        _ = shutdown_signal() => {
            println!("shutdown signal received, flushing state...");
            Ok(())
        }
    };

    {
        state.persist_all().await;
    }

    serve_result
}

/// Resolve when the process should shut down cleanly. Fires on
/// Ctrl-C (SIGINT) on any platform and additionally on SIGTERM
/// on Unix. SIGKILL is uncatchable by design; state integrity
/// under SIGKILL depends entirely on the per-handler
/// `persist_all` having already run, which it will have unless
/// the signal landed mid-poke.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}

#[derive(Debug, Default, serde::Deserialize)]
struct Raw {
    settlement_mode: Option<String>,
    chain_endpoint: Option<String>,
    tx_fee: Option<u64>,
    coinbase_timelock_min: Option<u64>,
    accept_timeout_secs: Option<u64>,
}

impl Raw {
    fn settlement_toml(&self) -> vesl_core::SettlementToml {
        vesl_core::SettlementToml {
            settlement_mode: self.settlement_mode.clone(),
            chain_endpoint: self.chain_endpoint.clone(),
            tx_fee: self.tx_fee,
            coinbase_timelock_min: self.coinbase_timelock_min,
            accept_timeout_secs: self.accept_timeout_secs,
        }
    }
}

fn load_toml(path: &std::path::Path) -> Raw {
    let raw: Raw = match std::fs::read_to_string(path) {
        Ok(contents) => toml::from_str(&contents).unwrap_or_else(|e| {
            eprintln!("warning: failed to parse {}: {e}", path.display());
            Raw::default()
        }),
        Err(_) => Raw::default(),
    };
    raw
}
