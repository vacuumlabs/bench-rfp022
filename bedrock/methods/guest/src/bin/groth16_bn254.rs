//! One Groth16/BN254 verification (Bedrock's Proof of Leadership has 10 public signals).
#![no_main]
use pol_guest::{check, decode, vk_x, Proof, Vk};
use risc0_zkvm::guest::env;
use substrate_bn::Fr;

risc0_zkvm::guest::entry!(main);

pub fn main() {
    let (vk, proof, inputs): (Vk, Proof, Vec<[u8; 32]>) = env::read();
    let c = env::cycle_count;
    let t0 = c();
    let (alpha, beta, gamma, delta, ic, a, b, cc) = decode(&vk, &proof);
    let xs: Vec<Fr> = inputs.iter().map(|x| Fr::from_slice(x).expect("scalar")).collect();
    let t1 = c();
    let vkx = vk_x(&ic, &xs);
    let t2 = c();
    let ok = check(alpha, beta, gamma, delta, vkx, a, b, cc);
    let t3 = c();
    env::commit(&ok);
    env::write(&(t1 - t0, t2 - t1, t3 - t2));
}
