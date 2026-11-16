//! Groth16 verification over BN254 as snarkjs/Circom defines it:
//! e(A, B) = e(alpha, beta) · e(vk_x, gamma) · e(C, delta), vk_x = IC_0 + Σ x_i · IC_i.
//! Points travel as big-endian coordinates (G1: x ‖ y, G2: x.c1 ‖ x.c0 ‖ y.c1 ‖ y.c0, the
//! EIP-197 order); decoding checks each point is on its curve (and in G2's subgroup).
use substrate_bn::{pairing, pairing_batch, AffineG1, AffineG2, Fq, Fq2, Fr, Group, Gt, G1, G2};
pub use substrate_bn::pairing as pair1;

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Vk {
    pub alpha: Vec<u8>,
    pub beta: Vec<u8>,
    pub gamma: Vec<u8>,
    pub delta: Vec<u8>,
    pub ic: Vec<Vec<u8>>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Proof {
    pub a: Vec<u8>,
    pub b: Vec<u8>,
    pub c: Vec<u8>,
}

fn fq(b: &[u8]) -> Fq {
    Fq::from_slice(b).expect("field element")
}

pub fn g1(b: &[u8]) -> G1 {
    AffineG1::new(fq(&b[..32]), fq(&b[32..64])).expect("G1 point").into()
}

pub fn g2(b: &[u8]) -> G2 {
    let x = Fq2::new(fq(&b[32..64]), fq(&b[..32]));
    let y = Fq2::new(fq(&b[96..128]), fq(&b[64..96]));
    AffineG2::new(x, y).expect("G2 point").into()
}

/// Returns (accepted, decode cycles input, ...) is left to the caller's counters.
pub fn decode(vk: &Vk, p: &Proof) -> (G1, G2, G2, G2, Vec<G1>, G1, G2, G1) {
    (g1(&vk.alpha), g2(&vk.beta), g2(&vk.gamma), g2(&vk.delta), vk.ic.iter().map(|b| g1(b)).collect(), g1(&p.a), g2(&p.b), g1(&p.c))
}

pub fn vk_x(ic: &[G1], inputs: &[Fr]) -> G1 {
    assert_eq!(ic.len(), inputs.len() + 1, "public input count");
    let mut acc = ic[0];
    for (p, x) in ic[1..].iter().zip(inputs) {
        acc = acc + *p * *x;
    }
    acc
}

pub fn check(alpha: G1, beta: G2, gamma: G2, delta: G2, vkx: G1, a: G1, b: G2, c: G1) -> bool {
    pairing_batch(&[(-a, b), (alpha, beta), (vkx, gamma), (c, delta)]) == Gt::one()
}

/// Trusted constants (a deployed verifier's key): coordinates only, no curve or subgroup check.
pub fn g1_const(b: &[u8]) -> G1 {
    G1::new(fq(&b[..32]), fq(&b[32..64]), Fq::one())
}

pub fn g2_const(b: &[u8]) -> G2 {
    G2::new(Fq2::new(fq(&b[32..64]), fq(&b[..32])), Fq2::new(fq(&b[96..128]), fq(&b[64..96])), Fq2::one())
}
