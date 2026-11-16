//! Second release path (DESIGN-DECISIONS.md §7), bench port: the release statement of
//! bench/zk, re-expressed for SP1 to measure its cost. The delivered second path is written
//! independently of the RISC Zero code; this port only sizes it.
use sha2::{Digest, Sha256};

pub const DEPTH: usize = 32;

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Input {
    pub cfg: [u8; 32],
    pub index: u64,
    pub siblings: Vec<[u8; 32]>,
    pub secret: [u8; 32],
    pub dest: [u8; 20],
    pub as_native: u8,
    pub token: [u8; 20],
    pub denom: [u8; 32],
    pub relayer: [u8; 20],
    pub relayer_fee: [u8; 32],
}

pub fn h(parts: &[&[u8]]) -> [u8; 32] {
    let mut s = Sha256::new();
    for p in parts {
        s.update(p);
    }
    s.finalize().into()
}

fn word_addr(a: &[u8; 20]) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a);
    w
}

/// The vault's ReleaseJournal, ABI-encoded (nine words).
pub fn release(i: &Input) -> Vec<u8> {
    assert_eq!(i.siblings.len(), DEPTH);
    let b = h(&[b"burn", &i.cfg, &i.token, &i.denom, &i.dest, &[i.as_native], &i.secret]);
    let mut cur = h(&[b"leaf", &i.token, &i.denom, &b]);
    let mut idx = i.index;
    for s in &i.siblings {
        cur = if idx & 1 == 0 { h(&[&cur, s]) } else { h(&[s, &cur]) };
        idx >>= 1;
    }
    let nullifier = h(&[b"bnul", &i.cfg, &i.secret, &b]);
    assert!(i.relayer_fee <= i.denom, "fee above amount");
    let mut nat = [0u8; 32];
    nat[31] = i.as_native;
    let mut j = Vec::with_capacity(9 * 32);
    for w in [i.cfg, cur, nullifier, word_addr(&i.dest), nat, word_addr(&i.token), i.denom, word_addr(&i.relayer), i.relayer_fee] {
        j.extend_from_slice(&w);
    }
    j
}

/// Relay batch input (bench/zk relay_range): pairing fields, burn-tree frontier, vault tip.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct RelayIn {
    pub cfg_pre: Vec<u8>,
    pub program_id: [u8; 32],
    pub next: u64,
    pub left: Vec<[u8; 32]>,
    pub prev_tip: [u8; 32],
    pub prev_height: u64,
    pub sizes: Vec<u32>,
}

const PREFIX: &[u8; 32] = b"/LEE/v0.3/Message/Block/\x00\x00\x00\x00\x00\x00\x00\x00";

fn zeros() -> [[u8; 32]; DEPTH + 1] {
    let mut z = [[0u8; 32]; DEPTH + 1];
    for k in 1..=DEPTH { z[k] = h(&[&z[k - 1], &z[k - 1]]); }
    z
}

fn root(next: u64, left: &[[u8; 32]], z: &[[u8; 32]; DEPTH + 1]) -> [u8; 32] {
    let (mut idx, mut cur) = (next, z[0]);
    for l in 0..DEPTH {
        cur = if idx & 1 == 1 { h(&[&left[l], &cur]) } else { h(&[&cur, &z[l]]) };
        idx >>= 1;
    }
    cur
}

fn word_u64(v: u64) -> [u8; 32] { let mut w = [0u8; 32]; w[24..].copy_from_slice(&v.to_be_bytes()); w }

/// One relay batch; returns the journal abi.encode(cfg, prevRoot, newRoot, prevTip, newTip,
/// newHeight, burns).
pub fn relay(i: &RelayIn, all: &[u8]) -> Vec<u8> {
    let cfg = h(&[&i.cfg_pre]);
    let z = zeros();
    let (mut next, mut left) = (i.next, i.left.clone());
    let prev_root = root(next, &left, &z);
    let (mut tip, mut burns, mut off) = (i.prev_tip, 0u64, 0usize);
    for &len in &i.sizes {
        let block = &all[off..off + len as usize];
        off += len as usize;
        assert_eq!(&block[..32], &tip, "block does not extend the relayed chain");
        tip = h(&[PREFIX, block]);
        for rec in block[32..].chunks(512) {
            if rec.len() >= 124 && &rec[..8] == b"BRGBURN1" && rec[8..40] == i.program_id {
                let mut cur = h(&[b"leaf", &rec[40..60], &rec[60..92], &rec[92..124]]);
                let mut idx = next;
                for l in 0..DEPTH {
                    if idx & 1 == 0 { left[l] = cur; cur = h(&[&cur, &z[l]]); } else { cur = h(&[&left[l], &cur]); }
                    idx >>= 1;
                }
                next += 1;
                burns += 1;
            }
        }
    }
    assert_eq!(off, all.len());
    let new_root = root(next, &left, &z);
    let mut j = Vec::with_capacity(7 * 32);
    for w in [cfg, prev_root, new_root, i.prev_tip, tip, word_u64(i.prev_height + i.sizes.len() as u64), word_u64(burns)] {
        j.extend_from_slice(&w);
    }
    j
}
