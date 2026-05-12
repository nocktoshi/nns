//! Path Y2 two-kernel scan cursor regression test.
//!
//! Supersedes the Phase 7 two-server `light_verify` freshness scenario
//! (`%advance-tip` + `/anchor` were removed). The same idea — two nodes
//! can diverge in what chain prefix they have scanned — is expressed
//! directly via `/scan-state` peeks after `%scan-block` pokes.

use std::sync::Arc;

use nns_vesl::chain::NNS_GENESIS_HEIGHT as H0;
use nns_vesl::kernel::{
    build_scan_block_poke, build_scan_state_peek, decode_scan_state, first_scan_block_done,
};
use nns_vesl::state::{AppState, SharedState};
use nockapp::kernel::boot;
use nockapp::kernel::boot::NockStackSize;
use nockapp::wire::{SystemWire, Wire};
use nockapp::NockApp;
use vesl_core::SettlementConfig;

fn kernel_jam() -> Vec<u8> {
    let path = std::env::var("NNS_KERNEL_JAM").unwrap_or_else(|_| "out.jam".to_string());
    std::fs::read(&path)
        .or_else(|_| std::fs::read("../out.jam"))
        .unwrap_or_else(|e| panic!("could not read kernel jam at {path} or ../out.jam: {e}"))
}

static TRACING_INIT: std::sync::Once = std::sync::Once::new();

async fn boot_kernel(name: &str) -> (tempfile::TempDir, SharedState) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut cli = boot::default_boot_cli(true);
    cli.stack_size = NockStackSize::Large;
    TRACING_INIT.call_once(|| {
        nns_vesl::prepare_tracy_for_host_cpu();
        let _ = boot::init_default_tracing(&cli);
    });
    let prover_hot_state = zkvm_jetpack::hot::produce_prover_hot_state();
    let app: NockApp = boot::setup(
        &kernel_jam(),
        cli,
        prover_hot_state.as_slice(),
        name,
        Some(tmp.path().to_path_buf()),
    )
    .await
    .expect("kernel boot");
    let state = Arc::new(AppState::new(
        app,
        tmp.path().to_path_buf(),
        SettlementConfig::local(),
    ));
    (tmp, state)
}

fn digest(seed: u8) -> Vec<u8> {
    vec![seed; 40]
}

fn seq_page_byte(h: u64) -> u8 {
    ((h - H0 + 1) & 0xff) as u8
}

/// Scan `blocks` many consecutive heights starting at [`H0`] (genesis cursor).
async fn scan_blocks_from_chain_origin(state: &SharedState, blocks: u64) {
    for i in 0..blocks {
        let h = H0 + i;
        let page = digest(seq_page_byte(h));
        let parent = if i == 0 {
            digest(0xEE)
        } else {
            digest(seq_page_byte(h - 1))
        };
        let poke = build_scan_block_poke(&parent, h, &page, &[], &[]);
        let effects = {
            let mut k = state.kernel.lock().await;
            k.poke(SystemWire.to_wire(), poke)
                .await
                .expect("scan-block poke")
        };
        let done = first_scan_block_done(&effects).expect("scan-block-done");
        assert_eq!(done.height, h);
        assert_eq!(done.digest, page);
    }
}

/// Continue scanning from `start_height+1` through `end_inclusive` (cursor
/// must already be at `start_height`).
async fn scan_blocks_continue(state: &SharedState, start_height: u64, end_inclusive: u64) {
    for h in (start_height + 1)..=end_inclusive {
        let page = digest(seq_page_byte(h));
        let parent = digest(seq_page_byte(h - 1));
        let poke = build_scan_block_poke(&parent, h, &page, &[], &[]);
        let effects = {
            let mut k = state.kernel.lock().await;
            k.poke(SystemWire.to_wire(), poke)
                .await
                .expect("scan-block poke")
        };
        let done = first_scan_block_done(&effects).expect("scan-block-done");
        assert_eq!(done.height, h);
        assert_eq!(done.digest, page);
    }
}

async fn peek_height(state: &SharedState) -> u64 {
    let mut k = state.kernel.lock().await;
    let slab = k
        .peek(build_scan_state_peek())
        .await
        .expect("scan-state peek");
    decode_scan_state(&slab)
        .expect("decode scan-state")
        .last_proved_height
}

#[tokio::test]
async fn two_kernels_divergent_scan_cursors() {
    let (_tmp_a, state_a) = boot_kernel("nns-pathy-a").await;
    scan_blocks_from_chain_origin(&state_a, 25).await;

    let (_tmp_b, state_b) = boot_kernel("nns-pathy-b").await;

    assert_eq!(peek_height(&state_a).await, H0 + 24);
    assert_eq!(peek_height(&state_b).await, 0);
}

#[tokio::test]
async fn scan_cursor_advances_when_catching_up() {
    let (_tmp, state) = boot_kernel("nns-pathy-catchup").await;
    assert_eq!(peek_height(&state).await, 0);

    scan_blocks_from_chain_origin(&state, 5).await;
    assert_eq!(peek_height(&state).await, H0 + 4);

    scan_blocks_continue(&state, H0 + 4, H0 + 7).await;
    assert_eq!(peek_height(&state).await, H0 + 7);
}
