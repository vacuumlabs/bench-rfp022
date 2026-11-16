//! LEZ burn relay, one batch: a contiguous range of LEZ blocks n+1..m after the vault's last
//! relayed block. For each block: its hash is SHA256(PREFIX || block bytes) as in
//! lez/common/src/block.rs, its previous-block hash is the block before (the first one's is
//! the vault's tip), so no block can be skipped; every bridge burn in it is appended to the
//! burn tree. The journal is ABI-encoded for the vault:
//!   abi.encode(cfg, prevRoot, newRoot, prevTip, newTip, newHeight, burns)
//! The blocks are synthetic but sized like real ones: bytes 0..32 hold the previous-block
//! hash, then fixed 512-byte records stand in for transactions (parsing a real block and
//! its receipts replaces the record scan; a real relay also skips transactions listed in
//! the block's reverted list). A burn record is
//!   "BRGBURN1" ‖ program id (32) ‖ token (20) ‖ denomination (32) ‖ B (32)
//! and counts only if written by this pairing's bridge program; its leaf binds B to the
//! token and denomination burned.
#![no_main]
use bridge_guests::{burn_leaf, word_u64, zeros, Frontier, Pairing};
use risc0_zkvm::guest::env;
use sha2::{Digest, Sha256};
use std::io::Read;

risc0_zkvm::guest::entry!(main);

const PREFIX: &[u8; 32] = b"/LEE/v0.3/Message/Block/\x00\x00\x00\x00\x00\x00\x00\x00";
const RECORD: usize = 512;
const BURN_TAG: &[u8; 8] = b"BRGBURN1";

pub fn main() {
    // pairing, frontier before, vault tip (hash, height), block sizes; then the blocks' bytes.
    let (pairing, mut frontier, prev_tip, prev_height, sizes): (Pairing, Frontier, [u8; 32], u64, Vec<u32>) = env::read();
    let cfg = pairing.cfg();
    let mut all = Vec::new();
    env::stdin().read_to_end(&mut all).unwrap();
    let c = env::cycle_count;
    let t0 = c();

    let z = zeros();
    let prev_root = frontier.root(&z);
    let mut tip = prev_tip;
    let mut burns = 0u64;
    let mut off = 0usize;
    let (mut hash_cycles, mut scan_cycles) = (0u64, 0u64);
    for &len in &sizes {
        let block = &all[off..off + len as usize];
        off += len as usize;
        let a = c();
        assert_eq!(&block[..32], &tip, "block does not extend the relayed chain");
        let mut s = Sha256::new();
        s.update(PREFIX);
        s.update(block);
        tip = s.finalize().into();
        let b = c();
        for rec in block[32..].chunks(RECORD) {
            if rec.len() >= 124 && &rec[..8] == BURN_TAG && rec[8..40] == pairing.program_id {
                let token: [u8; 20] = rec[40..60].try_into().unwrap();
                let denom: [u8; 32] = rec[60..92].try_into().unwrap();
                let b: [u8; 32] = rec[92..124].try_into().unwrap();
                frontier.append(burn_leaf(&token, &denom, &b), &z);
                burns += 1;
            }
        }
        hash_cycles += b - a;
        scan_cycles += c() - b;
    }
    assert_eq!(off, all.len());
    let new_root = frontier.root(&z);
    let new_height = prev_height + sizes.len() as u64;

    let mut j = Vec::with_capacity(7 * 32);
    for w in [cfg, prev_root, new_root, prev_tip, tip, word_u64(new_height), word_u64(burns)] {
        j.extend_from_slice(&w);
    }
    let t1 = c();
    env::commit_slice(&j);
    env::write(&(all.len() as u64, hash_cycles, scan_cycles, t1 - t0));
}
