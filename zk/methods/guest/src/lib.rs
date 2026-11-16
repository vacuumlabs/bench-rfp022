//! Shared pieces of the bridge guests: domain-separated SHA-256 and the burn tree.
use sha2::{Digest, Sha256};

pub const DEPTH: usize = 32;

/// The pairing a configuration id commits to. Guests recompute `cfg` from it and use its
/// fields, so a proof can only speak about this vault and this bridge program.
/// Must match BridgeVault's constructor: sha256("cfg" ‖ chain id (u64 BE) ‖ vault ‖ LEZ chain
/// id ‖ zone id ‖ program id ‖ RFP-021 instance).
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Pairing {
    pub evm_chain_id: u64,
    pub vault: [u8; 20],
    pub lez_chain_id: [u8; 32],
    pub zone_id: [u8; 32],
    pub program_id: [u8; 32],
    pub rfp021_instance: [u8; 32],
}

impl Pairing {
    pub fn cfg(&self) -> [u8; 32] {
        h(&[b"cfg", &self.evm_chain_id.to_be_bytes(), &self.vault, &self.lez_chain_id, &self.zone_id, &self.program_id, &self.rfp021_instance])
    }
}

/// Burn-tree leaf: binds the burn commitment B to the token and denomination the bridge
/// program actually burned (public fields of the burn record), so a B that opens to another
/// token or amount never matches a leaf.
pub fn burn_leaf(token: &[u8; 20], denom: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    h(&[b"leaf", token, denom, b])
}

pub fn h(parts: &[&[u8]]) -> [u8; 32] {
    let mut s = Sha256::new();
    for p in parts {
        s.update(p);
    }
    s.finalize().into()
}

pub fn node(l: &[u8; 32], r: &[u8; 32]) -> [u8; 32] {
    h(&[l, r])
}

/// Roots of all-empty subtrees, height 0..=DEPTH.
pub fn zeros() -> [[u8; 32]; DEPTH + 1] {
    let mut z = [[0u8; 32]; DEPTH + 1];
    for i in 1..=DEPTH {
        z[i] = node(&z[i - 1], &z[i - 1]);
    }
    z
}

/// Append-only Merkle tree state: next index and the left frontier per level.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Frontier {
    pub next: u64,
    pub left: Vec<[u8; 32]>, // DEPTH entries
}

impl Frontier {
    pub fn empty() -> Self {
        Frontier { next: 0, left: vec![[0u8; 32]; DEPTH] }
    }

    /// Append one leaf; returns the new root.
    pub fn append(&mut self, leaf: [u8; 32], z: &[[u8; 32]; DEPTH + 1]) -> [u8; 32] {
        let mut idx = self.next;
        let mut cur = leaf;
        for lvl in 0..DEPTH {
            if idx & 1 == 0 {
                self.left[lvl] = cur;
                cur = node(&cur, &z[lvl]);
            } else {
                cur = node(&self.left[lvl], &cur);
            }
            idx >>= 1;
        }
        self.next += 1;
        cur
    }

    /// Root of the tree as it stands (empty leaves after `next`).
    pub fn root(&self, z: &[[u8; 32]; DEPTH + 1]) -> [u8; 32] {
        let mut idx = self.next;
        let mut cur = z[0];
        for lvl in 0..DEPTH {
            cur = if idx & 1 == 1 { node(&self.left[lvl], &cur) } else { node(&cur, &z[lvl]) };
            idx >>= 1;
        }
        cur
    }

    pub fn digest(&self) -> [u8; 32] {
        let mut s = Sha256::new();
        s.update(self.next.to_be_bytes());
        for l in &self.left {
            s.update(l);
        }
        s.finalize().into()
    }
}

/// Root from a leaf and its sibling path.
pub fn root_from_path(leaf: [u8; 32], index: u64, siblings: &[[u8; 32]]) -> [u8; 32] {
    let mut cur = leaf;
    let mut idx = index;
    for s in siblings {
        cur = if idx & 1 == 0 { node(&cur, s) } else { node(s, &cur) };
        idx >>= 1;
    }
    cur
}

pub fn word_addr(a: &[u8; 20]) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a);
    w
}

pub fn word_u64(v: u64) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[24..].copy_from_slice(&v.to_be_bytes());
    w
}

pub fn word_u8(v: u8) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[31] = v;
    w
}
