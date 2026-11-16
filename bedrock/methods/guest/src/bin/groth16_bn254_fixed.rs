//! The same verification as a deployed verifier would run it: the key is a trusted constant
//! (no point validation) and e(alpha, beta) is precomputed (computed here outside the counters),
//! so the per-proof work is: validate A, B, C; vk_x; three pairings in one batch.
#![no_main]
use pol_guest::{g1, g1_const, g2, g2_const, pair1, vk_x, Proof, Vk};
use risc0_zkvm::guest::env;
use substrate_bn::{pairing_batch, Fr};

risc0_zkvm::guest::entry!(main);

pub fn main() {
    let (vk, proof, inputs): (Vk, Proof, Vec<[u8; 32]>) = env::read();
    let (alpha, beta, gamma, delta) = (g1_const(&vk.alpha), g2_const(&vk.beta), g2_const(&vk.gamma), g2_const(&vk.delta));
    let ic: Vec<_> = vk.ic.iter().map(|b| g1_const(b)).collect();
    let ab = pair1(alpha, beta); // a constant in a deployed verifier
    let c = env::cycle_count;
    let t0 = c();
    let (a, b, cc) = (g1(&proof.a), g2(&proof.b), g1(&proof.c));
    let xs: Vec<Fr> = inputs.iter().map(|x| Fr::from_slice(x).expect("scalar")).collect();
    let t1 = c();
    let vkx = vk_x(&ic, &xs);
    let t2 = c();
    let ok = pairing_batch(&[(a, b), (-vkx, gamma), (-cc, delta)]) == ab; // e(A,B)·e(vk_x,γ)^-1·e(C,δ)^-1 = e(α,β)
    let t3 = c();
    env::commit(&ok);
    env::write(&(t1 - t0, t2 - t1, t3 - t2));
}
