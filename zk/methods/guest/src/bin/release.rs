//! Release proof (user, outbound): the leaf H(token, denom, B) is in the burn tree under
//! root T, so B was burned for exactly this token and denomination; B opens with the user's
//! seed-derived secret to (dest, asset, token, denom); the nullifier is derived from the
//! secret and B. The journal is the vault's ReleaseJournal,
//! ABI-encoded, so the vault acts only on proved fields.
#![no_main]
use bridge_guests::{burn_leaf, h, root_from_path, word_addr, word_u8, DEPTH};
use risc0_zkvm::guest::env;
use serde::Deserialize;

risc0_zkvm::guest::entry!(main);

#[derive(Deserialize)]
struct Input {
    cfg: [u8; 32],
    index: u64,
    siblings: Vec<[u8; 32]>,
    secret: [u8; 32],
    dest: [u8; 20],
    as_native: u8,
    token: [u8; 20],
    denom: [u8; 32],
    relayer: [u8; 20],
    relayer_fee: [u8; 32],
}

pub fn main() {
    let i: Input = env::read();
    let t0 = env::cycle_count();
    assert_eq!(i.siblings.len(), DEPTH);
    let b = h(&[b"burn", &i.cfg, &i.token, &i.denom, &i.dest, &[i.as_native], &i.secret]);
    let root = root_from_path(burn_leaf(&i.token, &i.denom, &b), i.index, &i.siblings);
    let nullifier = h(&[b"bnul", &i.cfg, &i.secret, &b]);
    assert!(i.relayer_fee <= i.denom, "fee above amount");
    // abi.encode(ReleaseJournal): nine 32-byte words.
    let mut j = Vec::with_capacity(9 * 32);
    for w in [i.cfg, root, nullifier, word_addr(&i.dest), word_u8(i.as_native), word_addr(&i.token), i.denom, word_addr(&i.relayer), i.relayer_fee] {
        j.extend_from_slice(&w);
    }
    let t1 = env::cycle_count();
    env::commit_slice(&j);
    // Cycle count goes to stderr-free host via a second, ignored channel: the journal stays exact.
    env::write(&(t1 - t0));
}
