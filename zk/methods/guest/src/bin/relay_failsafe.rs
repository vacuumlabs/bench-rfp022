//! LEZ relay with the failsafe's extra outputs (proposal §9): the same batch as
//! relay_range, plus
//!   - every claim's nullifier N appended to a second append-only tree (the claim-nullifier
//!     tree an inbound refund proves non-membership against, up to the freeze height);
//!   - cumulative claim and burn counts per (token, denomination), committed as one hash, which
//!     the vault's settle step opens to compute the pro-rata ratio.
//! A claim record is "BRGCLAIM" ‖ program id (32) ‖ token (20) ‖ denomination (32) ‖ N (32).
//! Journal: abi.encode(cfg, prevRoot, newRoot, prevClaimRoot, newClaimRoot, prevCounts,
//!   newCounts, prevTip, newTip, newHeight, burns, claims)
#![no_main]
use bridge_guests::{burn_leaf, h, word_u64, zeros, Frontier, Pairing};
use risc0_zkvm::guest::env;
use sha2::{Digest, Sha256};
use std::io::Read;

risc0_zkvm::guest::entry!(main);

const PREFIX: &[u8; 32] = b"/LEE/v0.3/Message/Block/\x00\x00\x00\x00\x00\x00\x00\x00";
const RECORD: usize = 512;
const BURN_TAG: &[u8; 8] = b"BRGBURN1";
const CLAIM_TAG: &[u8; 8] = b"BRGCLAIM";

type Counts = Vec<([u8; 20], [u8; 32], u64, u64)>; // token, denom, claims, burns

fn counts_hash(c: &Counts) -> [u8; 32] {
    let mut s = Sha256::new();
    s.update(b"counts");
    for (t, d, cl, b) in c {
        s.update(t);
        s.update(d);
        s.update(cl.to_be_bytes());
        s.update(b.to_be_bytes());
    }
    s.finalize().into()
}

fn bump(c: &mut Counts, token: &[u8; 20], denom: &[u8; 32], claim: bool) {
    let e = c.iter_mut().find(|e| &e.0 == token && &e.1 == denom).expect("unregistered token or denomination");
    if claim { e.2 += 1 } else { e.3 += 1 }
}

pub fn main() {
    let (pairing, mut frontier, mut claim_frontier, mut counts, prev_tip, prev_height, sizes): (
        Pairing, Frontier, Frontier, Counts, [u8; 32], u64, Vec<u32>,
    ) = env::read();
    let cfg = pairing.cfg();
    let mut all = Vec::new();
    env::stdin().read_to_end(&mut all).unwrap();
    let c = env::cycle_count;
    let t0 = c();

    let z = zeros();
    let prev_root = frontier.root(&z);
    let prev_claim_root = claim_frontier.root(&z);
    let prev_counts = counts_hash(&counts);
    let mut tip = prev_tip;
    let (mut burns, mut claims) = (0u64, 0u64);
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
            if rec.len() < 124 || rec[8..40] != pairing.program_id {
                continue;
            }
            let token: [u8; 20] = rec[40..60].try_into().unwrap();
            let denom: [u8; 32] = rec[60..92].try_into().unwrap();
            let v: [u8; 32] = rec[92..124].try_into().unwrap();
            if &rec[..8] == BURN_TAG {
                frontier.append(burn_leaf(&token, &denom, &v), &z);
                bump(&mut counts, &token, &denom, false);
                burns += 1;
            } else if &rec[..8] == CLAIM_TAG {
                claim_frontier.append(h(&[b"cnul", &token, &denom, &v]), &z);
                bump(&mut counts, &token, &denom, true);
                claims += 1;
            }
        }
        hash_cycles += b - a;
        scan_cycles += c() - b;
    }
    assert_eq!(off, all.len());
    let new_root = frontier.root(&z);
    let new_claim_root = claim_frontier.root(&z);
    let new_counts = counts_hash(&counts);
    let new_height = prev_height + sizes.len() as u64;

    let mut j = Vec::with_capacity(12 * 32);
    for w in [cfg, prev_root, new_root, prev_claim_root, new_claim_root, prev_counts, new_counts, prev_tip, tip,
        word_u64(new_height), word_u64(burns), word_u64(claims)] {
        j.extend_from_slice(&w);
    }
    let t1 = c();
    env::commit_slice(&j);
    env::write(&(all.len() as u64, hash_cycles, scan_cycles, t1 - t0));
}
