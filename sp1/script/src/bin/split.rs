//! SP1 Groth16 proof of the release port in three separate processes, so that no stage holds
//! another stage's memory (on a 16 GB machine the one-process SDK path ran out of memory):
//!   split compress  -> ../results/split/compressed.bin   (SDK: core + compressed STARK)
//!   split wrap      -> ../results/split/wrap.bin         (node: shrink + wrap to BN254)
//!   split groth16   -> ../contracts/fixtures/release-groth16.json (gnark only)
//! The witness and proof assembly in `groth16` follow sp1-prover 6.10.0's `run_groth16`.
use sp1_bridge_lib::{h, Input, DEPTH};
use sp1_sdk::{blocking::{ProveRequest, Prover, ProverClient}, include_elf, Elf, HashableKey, ProvingKey, SP1Proof, SP1ProofWithPublicValues, SP1Stdin};
use std::{borrow::Borrow, time::Instant};

const ELF: Elf = include_elf!("release-program");
const DIR: &str = "../results/split";

fn seeded(tag: &str, i: u64) -> [u8; 32] { h(&[tag.as_bytes(), &i.to_be_bytes()]) }
fn u256(v: u128) -> [u8; 32] { let mut w = [0u8; 32]; w[16..].copy_from_slice(&v.to_be_bytes()); w }
fn addr(s: &str) -> [u8; 20] { hex::decode(s.trim_start_matches("0x")).unwrap().try_into().unwrap() }

fn release_input() -> Input {
    let cfg = h(&[b"cfg", &1u64.to_be_bytes(), &addr("0x2e234DAe75C793f67A35089C9d99245E1C58470b"),
        &h(&[b"lez"]), &h(&[b"zone"]), &h(&[b"program"]), &h(&[b"rfp021"])]);
    Input {
        cfg,
        index: 1_000_000,
        siblings: (0..DEPTH as u64).map(|k| seeded("sibling", k)).collect(),
        secret: seeded("secret", 0),
        dest: addr("0x000000000000000000000000000000000000dEaD"),
        as_native: 0,
        token: addr("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
        denom: u256(1_000_000_000),
        relayer: addr("0x0000000000000000000000000000000000000B0b"),
        relayer_fee: u256(1_000_000),
    }
}

fn main() {
    sp1_sdk::utils::setup_logger();
    std::fs::create_dir_all(DIR).unwrap();
    let stage = std::env::args().nth(1).expect("stage: compress | wrap | groth16");
    let t = Instant::now();
    match stage.as_str() {
        "compress" => {
            let client = ProverClient::builder().cpu().build();
            let pk = client.setup(ELF).expect("setup");
            let mut stdin = SP1Stdin::new();
            stdin.write(&release_input());
            let proof = client.prove(&pk, stdin).compressed().run().expect("prove");
            client.verify(&proof, pk.verifying_key(), None).expect("verify");
            proof.save(format!("{DIR}/compressed.bin")).unwrap();
            std::fs::write(format!("{DIR}/vkey.txt"), pk.verifying_key().bytes32()).unwrap();
        }
        "wrap" => {
            let proof = SP1ProofWithPublicValues::load(format!("{DIR}/compressed.bin")).unwrap();
            let rt = tokio::runtime::Runtime::new().unwrap();
            let wrap = rt.block_on(async {
                let node = sp1_prover::worker::SP1LocalNodeBuilder::from_worker_client_builder(
                    sp1_prover::worker::cpu_worker_builder()).build().await.unwrap();
                node.shrink_wrap(&proof.proof).await.unwrap()
            });
            std::fs::write(format!("{DIR}/wrap.bin"), bincode::serialize(&wrap).unwrap()).unwrap();
        }
        "groth16" => groth16(),
        _ => panic!("unknown stage"),
    }
    println!("stage {stage}: {:.1}s", t.elapsed().as_secs_f64());
}

fn groth16() {
    use slop_algebra::{AbstractField, PrimeField, PrimeField32};
    use slop_bn254::Bn254Fr;
    use sp1_hypercube::{koalabears_to_bn254, SP1PcsProofOuter, SP1WrapProof};
    use sp1_primitives::{SP1Field, SP1OuterGlobalContext};
    use sp1_recursion_circuit::{machine::SP1ShapedWitnessValues,
        utils::{koalabear_bytes_to_bn254, koalabears_proof_nonce_to_bn254, words_to_bytes},
        witness::{OuterWitness, Witnessable}};
    use sp1_recursion_executor::RecursionPublicValues;
    use sp1_recursion_gnark_ffi::Groth16Bn254Prover;

    let compressed = SP1ProofWithPublicValues::load(format!("{DIR}/compressed.bin")).unwrap();
    let wrap: SP1WrapProof<SP1OuterGlobalContext, SP1PcsProofOuter> =
        bincode::deserialize(&std::fs::read(format!("{DIR}/wrap.bin")).unwrap()).unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let build_dir = rt.block_on(sp1_prover::build::try_build_groth16_artifacts_dir(&wrap.vk, &wrap.proof)).unwrap();

    let SP1WrapProof { vk, proof } = wrap;
    let input = SP1ShapedWitnessValues { vks_and_proofs: vec![(vk, proof.clone())], is_complete: true };
    let pv: &RecursionPublicValues<SP1Field> = proof.public_values.as_slice().borrow();
    let vkey_hash = koalabears_to_bn254(&pv.sp1_vk_digest);
    let cvd: [SP1Field; 32] = words_to_bytes(&pv.committed_value_digest).try_into().unwrap();
    let committed_values_digest = koalabear_bytes_to_bn254(&cvd);
    let exit_code = Bn254Fr::from_canonical_u32(pv.exit_code.as_canonical_u32());
    let proof_nonce = koalabears_proof_nonce_to_bn254(&pv.proof_nonce);
    let vk_root = koalabears_to_bn254(&pv.vk_root);
    let mut witness = OuterWitness::default();
    input.write(&mut witness);
    witness.write_committed_values_digest(committed_values_digest);
    witness.write_vkey_hash(vkey_hash);
    witness.write_exit_code(exit_code);
    witness.write_vk_root(vk_root);
    witness.write_proof_nonce(proof_nonce);

    let prover = Groth16Bn254Prover::new();
    let g16 = prover.prove(witness, &build_dir);
    prover.verify(&g16, &vkey_hash.as_canonical_biguint(), &committed_values_digest.as_canonical_biguint(),
        &exit_code.as_canonical_biguint(), &vk_root.as_canonical_biguint(), &proof_nonce.as_canonical_biguint(), &build_dir)
        .expect("groth16 verify");

    let out = SP1ProofWithPublicValues::new(SP1Proof::Groth16(g16), compressed.public_values.clone(), compressed.sp1_version.clone());
    let vkey = std::fs::read_to_string(format!("{DIR}/vkey.txt")).unwrap();
    let public_values = format!("0x{}", hex::encode(out.public_values.as_slice()));
    let r0: serde_json::Value = serde_json::from_str(&std::fs::read_to_string("../../vault/proofs/release.json").unwrap()).unwrap();
    let fixture = serde_json::json!({ "vkey": vkey, "publicValues": public_values, "proof": format!("0x{}", hex::encode(out.bytes())) });
    std::fs::create_dir_all("../contracts/fixtures").unwrap();
    std::fs::write("../contracts/fixtures/release-groth16.json", serde_json::to_string_pretty(&fixture).unwrap()).unwrap();
    println!("fixture written; journal_matches_risc0={}", Some(public_values.as_str()) == r0["journal"].as_str());
}
