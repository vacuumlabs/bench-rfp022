//! Cost of verifying one Bedrock Proof of Leadership inside RISC Zero 3.1.
//!   pol-bench inputs            writes results/inputs/{synthetic,pol_vk}.bin
//!   pol-bench execute <input>   cycles
//!   pol-bench prove <input>     succinct STARK, timed
//! synthetic: a valid Groth16 tuple for a BN254 key with 10 public inputs, built from a known
//!   trapdoor (A = a·G1, B = b·G2, C chosen so the verification equation holds). The verifier
//!   does exactly the work it does for a real proof: point decoding, 10-term vk_x, 4 pairings.
//! pol_vk: Bedrock's real PoL verifying key (circuits v0.8.0-rc.1, parsed from its snarkjs
//!   JSON, every point checked on its curve) with the synthetic proof: same work, rejected.
use std::{fs, time::Instant};

use pol_guest::{check, vk_x, Proof, Vk};
use pol_methods::{GROTH16_BN254_ELF, GROTH16_BN254_FIXED_ELF, GROTH16_BN254_FIXED_ID, GROTH16_BN254_ID};
use risc0_zkvm::{default_executor, default_prover, ExecutorEnv, ProverOpts};
use substrate_bn::{AffineG1, AffineG2, Fq, Fq2, Fr, Group, G1, G2};

fn fr(n: u64) -> Fr {
    // deterministic, spread over the field
    let mut x = Fr::from_str(&format!("{}", n.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)).unwrap();
    for _ in 0..5 {
        x = x * x + Fr::from_str("7").unwrap();
    }
    x
}

fn be(f: Fq) -> [u8; 32] {
    let mut b = [0u8; 32];
    f.to_big_endian(&mut b).unwrap();
    b
}

fn enc1(p: G1) -> Vec<u8> {
    let a = AffineG1::from_jacobian(p).unwrap();
    [be(a.x()), be(a.y())].concat()
}

fn enc2(p: G2) -> Vec<u8> {
    let a = AffineG2::from_jacobian(p).unwrap();
    [be(a.x().imaginary()), be(a.x().real()), be(a.y().imaginary()), be(a.y().real())].concat()
}

fn fr_bytes(x: Fr) -> [u8; 32] {
    let mut b = [0u8; 32];
    x.into_u256().to_big_endian(&mut b).unwrap();
    b
}

fn words<T: serde::Serialize>(v: &T) -> Vec<u8> {
    risc0_zkvm::serde::to_vec(v).unwrap().iter().flat_map(|w| w.to_le_bytes()).collect()
}

fn dec(s: &serde_json::Value) -> Fq {
    Fq::from_str(s.as_str().unwrap()).expect("decimal coordinate")
}

fn snark_g1(v: &serde_json::Value) -> Vec<u8> {
    let p: G1 = AffineG1::new(dec(&v[0]), dec(&v[1])).expect("on curve").into();
    enc1(p)
}

fn snark_g2(v: &serde_json::Value) -> Vec<u8> {
    let x = Fq2::new(dec(&v[0][0]), dec(&v[0][1]));
    let y = Fq2::new(dec(&v[1][0]), dec(&v[1][1]));
    let p: G2 = AffineG2::new(x, y).expect("on curve, in subgroup").into();
    enc2(p)
}

fn inputs() {
    const N: usize = 10;
    let (al, bt, gm, dl) = (fr(1), fr(2), fr(3), fr(4));
    let ics: Vec<Fr> = (0..=N as u64).map(|i| fr(100 + i)).collect();
    let xs: Vec<Fr> = (0..N as u64).map(|i| fr(200 + i)).collect();
    let (a, b) = (fr(5), fr(6));
    let mut s = ics[0];
    for (ic, x) in ics[1..].iter().zip(&xs) {
        s = s + *ic * *x;
    }
    // a·b = α·β + s·γ + c·δ
    let c = (a * b - al * bt - s * gm) * dl.inverse().unwrap();
    let (g1, g2) = (G1::one(), G2::one());
    let vk = Vk { alpha: enc1(g1 * al), beta: enc2(g2 * bt), gamma: enc2(g2 * gm), delta: enc2(g2 * dl), ic: ics.iter().map(|i| enc1(g1 * *i)).collect() };
    let proof = Proof { a: enc1(g1 * a), b: enc2(g2 * b), c: enc1(g1 * c) };
    let xb: Vec<[u8; 32]> = xs.iter().map(|x| fr_bytes(*x)).collect();
    // host check of the same code the guest runs
    let d = pol_guest::decode(&vk, &proof);
    assert!(check(d.0, d.1, d.2, d.3, vk_x(&d.4, &xs), d.5, d.6, d.7), "synthetic proof must verify");
    fs::create_dir_all("results/inputs").unwrap();
    fs::write("results/inputs/synthetic.bin", words(&(vk.clone(), proof.clone(), xb.clone()))).unwrap();

    let j: serde_json::Value = serde_json::from_str(&fs::read_to_string("fixtures/pol_verification_key_v0.8.0-rc.1.json").unwrap()).unwrap();
    assert_eq!(j["protocol"], "groth16");
    assert_eq!(j["curve"], "bn128");
    let n = j["nPublic"].as_u64().unwrap() as usize;
    let pol = Vk {
        alpha: snark_g1(&j["vk_alpha_1"]),
        beta: snark_g2(&j["vk_beta_2"]),
        gamma: snark_g2(&j["vk_gamma_2"]),
        delta: snark_g2(&j["vk_delta_2"]),
        ic: j["IC"].as_array().unwrap().iter().map(snark_g1).collect(),
    };
    assert_eq!(pol.ic.len(), n + 1);
    let d = pol_guest::decode(&pol, &proof);
    assert!(!check(d.0, d.1, d.2, d.3, vk_x(&d.4, &xs[..n]), d.5, d.6, d.7));
    fs::write("results/inputs/pol_vk.bin", words(&(pol, proof, xb[..n].to_vec()))).unwrap();
    println!("PoL key: groth16 bn128, nPublic {n}; wrote results/inputs/{{synthetic,pol_vk}}.bin");
}

fn env(input: &str) -> ExecutorEnv<'static> {
    ExecutorEnv::builder().write_slice(&fs::read(input).unwrap()).build().unwrap()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(String::as_str) {
        Some("inputs") => inputs(),
        Some("execute") => {
            let elf = if a.get(3).map(String::as_str) == Some("fixed") { GROTH16_BN254_FIXED_ELF } else { GROTH16_BN254_ELF };
            let mut out = Vec::new();
            let e = ExecutorEnv::builder().write_slice(&fs::read(&a[2]).unwrap()).stdout(&mut out).build().unwrap();
            let info = default_executor().execute(e, elf).expect("execute");
            let user: u64 = info.segments.iter().map(|s| s.cycles as u64).sum();
            let padded: u64 = info.segments.iter().map(|s| 1u64 << s.po2).sum();
            let ok: bool = info.journal.decode().unwrap();
            let (dc, msm, pair): (u64, u64, u64) = risc0_zkvm::serde::from_slice(&out.chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect::<Vec<u32>>()).unwrap();
            println!("EXEC {} {} accepted={ok} user_cycles={user} padded_cycles={padded} segments={} decode={dc} vk_x={msm} pairing_check={pair}", a[2], a.get(3).map(String::as_str).unwrap_or("generic"), info.segments.len());
        }
        Some("prove") => {
            let (elf, id) = if a.get(3).map(String::as_str) == Some("fixed") { (GROTH16_BN254_FIXED_ELF, GROTH16_BN254_FIXED_ID) } else { (GROTH16_BN254_ELF, GROTH16_BN254_ID) };
            let t = Instant::now();
            let info = default_prover().prove_with_opts(env(&a[2]), elf, &ProverOpts::succinct()).expect("prove");
            let secs = t.elapsed().as_secs_f64();
            info.receipt.verify(id).expect("verify");
            println!("PROVED {} total_cycles={} user_cycles={} segments={} prove_s={secs:.1}", a[2], info.stats.total_cycles, info.stats.user_cycles, info.stats.segments);
        }
        _ => eprintln!("usage: pol-bench inputs | execute <input> | prove <input>"),
    }
}
