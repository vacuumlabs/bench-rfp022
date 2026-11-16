//! Execute (no proving) the SP1 release port on the same input as bench/zk's release proof and
//! compare its journal with the RISC Zero journal in ../vault/proofs/release.json.
use sp1_bridge_lib::{h, Input, RelayIn, DEPTH};
use sp1_sdk::{blocking::{ProveRequest, Prover, ProverClient}, include_elf, Elf, HashableKey, ProvingKey, SP1Stdin};
use std::time::Instant;

const ELF: Elf = include_elf!("release-program");
const RELAY_ELF: Elf = include_elf!("relay-program");

fn seeded(tag: &str, i: u64) -> [u8; 32] { h(&[tag.as_bytes(), &i.to_be_bytes()]) }
fn u256(v: u128) -> [u8; 32] { let mut w = [0u8; 32]; w[16..].copy_from_slice(&v.to_be_bytes()); w }
fn addr(s: &str) -> [u8; 20] { hex::decode(s.trim_start_matches("0x")).unwrap().try_into().unwrap() }

fn main() {
    // Pairing of bench/zk (chain 1, vault 0x2e23…470b), same cfg formula as BridgeVault.
    let cfg = h(&[b"cfg", &1u64.to_be_bytes(), &addr("0x2e234DAe75C793f67A35089C9d99245E1C58470b"),
        &h(&[b"lez"]), &h(&[b"zone"]), &h(&[b"program"]), &h(&[b"rfp021"])]);
    let i = Input {
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
    };
    let mut stdin = SP1Stdin::new();
    stdin.write(&i);
    let client = ProverClient::builder().cpu().build();
    let mode = std::env::args().nth(1).unwrap_or_default();
    if mode == "prove-release" {
        // Real proof: core, then compressed and Groth16-wrapped for Ethereum.
        let t = Instant::now();
        let pk = client.setup(ELF).expect("setup");
        println!("setup: {:.1}s", t.elapsed().as_secs_f64());
        let t = Instant::now();
        let proof = client.prove(&pk, stdin).groth16().run().expect("prove");
        println!("groth16 proof: {:.1}s", t.elapsed().as_secs_f64());
        client.verify(&proof, pk.verifying_key(), None).expect("verify");
        let fixture = serde_json::json!({
            "vkey": pk.verifying_key().bytes32().to_string(),
            "publicValues": format!("0x{}", hex::encode(proof.public_values.as_slice())),
            "proof": format!("0x{}", hex::encode(proof.bytes())),
        });
        std::fs::create_dir_all("../contracts/fixtures").unwrap();
        std::fs::write("../contracts/fixtures/release-groth16.json", serde_json::to_string_pretty(&fixture).unwrap()).unwrap();
        println!("fixture written; journal_matches_risc0={}", fixture["publicValues"].as_str() == serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string("../../vault/proofs/release.json").unwrap()).unwrap()["journal"].as_str());
        return;
    }
    let (out, report) = client.execute(ELF, stdin).run().unwrap();
    let got = format!("0x{}", hex::encode(out.as_slice()));
    let r0: serde_json::Value = serde_json::from_str(&std::fs::read_to_string("../../vault/proofs/release.json").unwrap()).unwrap();
    println!("SP1 release: instructions={} journal_matches_risc0={}", report.total_instruction_count(), Some(got.as_str()) == r0["journal"].as_str());

    // Relay batches, the same synthetic LEZ blocks as bench/zk's range inputs.
    let program_id = h(&[b"program"]);
    let mut cfg_pre = b"cfg".to_vec();
    for part in [&1u64.to_be_bytes()[..], &addr("0x2e234DAe75C793f67A35089C9d99245E1C58470b"), &h(&[b"lez"]), &h(&[b"zone"]), &program_id, &h(&[b"rfp021"])] {
        cfg_pre.extend_from_slice(part);
    }
    let expected = std::fs::read_to_string("../r0_relay_journals.txt").unwrap();
    let tip0 = seeded("lez-tip", 0);
    for (name, n, size) in [("range_1x64k", 1usize, 64 * 1024usize), ("range_16x64k", 16, 64 * 1024), ("range_1x1m", 1, 1024 * 1024)] {
        let (mut tip, mut bytes, mut sizes) = (tip0, Vec::new(), Vec::new());
        for k in 0..n {
            let mut block = vec![0u8; size];
            block[..32].copy_from_slice(&tip);
            for (t, rec) in block[32..].chunks_mut(512).enumerate() {
                rec[..32].copy_from_slice(&seeded("tx", (k * 100_000 + t) as u64));
            }
            for b in 0..4usize {
                let off = 32 + (b * (size - 32) / 4) / 512 * 512;
                block[off..off + 8].copy_from_slice(b"BRGBURN1");
                block[off + 8..off + 40].copy_from_slice(&program_id);
                block[off + 40..off + 60].copy_from_slice(&addr("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"));
                block[off + 60..off + 92].copy_from_slice(&u256(1_000_000_000));
                block[off + 92..off + 124].copy_from_slice(&seeded("burn", (k * 4 + b) as u64));
            }
            tip = h(&[b"/LEE/v0.3/Message/Block/\x00\x00\x00\x00\x00\x00\x00\x00", &block]);
            sizes.push(size as u32);
            bytes.extend_from_slice(&block);
        }
        let ri = RelayIn { cfg_pre: cfg_pre.clone(), program_id, next: 1_000_000, left: (0..DEPTH as u64).map(|k| seeded("frontier", k)).collect(), prev_tip: tip0, prev_height: 5_000_000, sizes };
        let mut stdin = SP1Stdin::new();
        stdin.write(&ri);
        stdin.write_vec(bytes);
        if mode == "prove-relay" {
            if std::env::args().nth(2).as_deref() != Some(name) { continue; }
            let pk = client.setup(RELAY_ELF).expect("setup");
            let t = Instant::now();
            let proof = client.prove(&pk, stdin).groth16().run().expect("prove");
            println!("SP1 relay {name} groth16 proof: {:.1}s", t.elapsed().as_secs_f64());
            client.verify(&proof, pk.verifying_key(), None).expect("verify");
            let got = format!("0x{}", hex::encode(proof.public_values.as_slice()));
            let want = expected.lines().find(|l| l.starts_with(name)).and_then(|l| l.split_whitespace().nth(1)).unwrap();
            println!("journal_matches_risc0={} proof_bytes={}", got == want, proof.bytes().len());
            continue;
        }
        let (out, report) = client.execute(RELAY_ELF, stdin).run().unwrap();
        let got = hex::encode(out.as_slice());
        let want = expected.lines().find(|l| l.starts_with(name)).and_then(|l| l.split_whitespace().nth(1)).unwrap();
        println!("SP1 relay {name}: instructions={} journal_matches_risc0={}", report.total_instruction_count(), want == format!("0x{got}"));
    }
}
