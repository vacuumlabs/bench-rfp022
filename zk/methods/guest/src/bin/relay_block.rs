//! Relay cost per LEZ block: recompute the block hash over the whole block
//! (SHA256(PREFIX || borsh(block data)), as lez/common/src/block.rs), scan its
//! transactions for bridge burns and append each burn commitment to the burn tree.
//! The block here is synthetic but sized like a real one; parsing a real
//! LeeTransaction list is replaced by a fixed-size record scan.
#![no_main]
use bridge_guests::{zeros, Frontier};
use risc0_zkvm::guest::env;
use sha2::{Digest, Sha256};
use std::io::Read;

risc0_zkvm::guest::entry!(main);

const PREFIX: &[u8; 32] = b"/LEE/v0.3/Message/Block/\x00\x00\x00\x00\x00\x00\x00\x00";
const RECORD: usize = 512; // stand-in transaction size
const BURN_TAG: &[u8; 8] = b"BRGBURN1";

pub fn main() {
    let (mut frontier, burns_expected): (Frontier, u32) = env::read();
    let mut block = Vec::new();
    env::stdin().read_to_end(&mut block).unwrap();
    let c = env::cycle_count;

    let t0 = c();
    let mut s = Sha256::new();
    s.update(PREFIX);
    s.update(&block);
    let block_hash: [u8; 32] = s.finalize().into();
    let t1 = c();

    let z = zeros();
    let mut burns = 0u32;
    let mut root = [0u8; 32];
    for rec in block.chunks(RECORD) {
        if rec.len() >= 40 && &rec[..8] == BURN_TAG {
            let b: [u8; 32] = rec[8..40].try_into().unwrap();
            root = frontier.append(b, &z);
            burns += 1;
        }
    }
    let t2 = c();
    assert_eq!(burns, burns_expected);
    env::commit(&(block_hash, root, frontier.digest(), burns));
    env::write(&(block.len() as u64, t1 - t0, t2 - t1));
}
