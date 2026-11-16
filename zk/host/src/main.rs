//! Ethereum-side bridge proofs on RISC Zero 3.1.0.
//!   bridge-bench inputs                      write results/inputs/*.bin
//!   bridge-bench execute <prog> <input>      cycles (no proving)
//!   bridge-bench prove <prog> <input>        succinct STARK, timed
//!   bridge-bench groth16 <prog> <input>      Groth16 for Ethereum, timed; writes ../vault/proofs/<prog>.json
//! Set RISC0_SERVER_PATH to the 3.1.0 r0vm.
use std::{fs, time::Instant};

use bridge_guests::{burn_leaf, root_from_path, Frontier, Pairing, DEPTH};
use bridge_methods::{RELAY_BLOCK_ELF, RELAY_BLOCK_ID, RELAY_FAILSAFE_ELF, RELAY_FAILSAFE_ID, RELAY_RANGE_ELF, RELAY_RANGE_ID, RELEASE_ELF, RELEASE_ID};
use risc0_zkvm::{default_executor, default_prover, ExecutorEnv, ProverOpts, Receipt};
use serde::Serialize;

#[derive(Serialize)]
struct ReleaseIn {
    cfg: [u8; 32],
    index: u64,
    siblings: Vec<[u8; 32]>,
    secret: [u8; 32],
    dest: [u8; 20],
    as_native: u8,
    token: [u8; 20],
    denom: [u8; 32],
    relayer: [u8; 20],
    relayer_fee: [u8; 32],
}

fn seeded(tag: &str, i: u64) -> [u8; 32] {
    bridge_guests::h(&[tag.as_bytes(), &i.to_be_bytes()])
}

fn u256(v: u128) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[16..].copy_from_slice(&v.to_be_bytes());
    w
}

fn addr(hexs: &str) -> [u8; 20] {
    hex::decode(hexs.trim_start_matches("0x")).unwrap().try_into().unwrap()
}

fn words<T: Serialize>(v: &T) -> Vec<u8> {
    risc0_zkvm::serde::to_vec(v).unwrap().iter().flat_map(|w| w.to_le_bytes()).collect()
}

fn write_inputs() {
    fs::create_dir_all("results/inputs").unwrap();
    // Release of 1,000 USDC with a 1 USDC relayer fee, burn at index 1,000,000 of the tree.
    // The pairing matches vault/test/RealProof.t.sol: chain 1, the vault the test deploys at
    // 0x2e234DAe75C793f67A35089C9d99245E1C58470b (second contract created by the test).
    let pairing = Pairing {
        evm_chain_id: 1,
        vault: addr("0x2e234DAe75C793f67A35089C9d99245E1C58470b"),
        lez_chain_id: bridge_guests::h(&[b"lez"]),
        zone_id: bridge_guests::h(&[b"zone"]),
        program_id: bridge_guests::h(&[b"program"]),
        rfp021_instance: bridge_guests::h(&[b"rfp021"]),
    };
    let cfg = pairing.cfg();
    let r = ReleaseIn {
        cfg,
        index: 1_000_000,
        siblings: (0..DEPTH as u64).map(|i| seeded("sibling", i)).collect(),
        secret: seeded("secret", 0),
        dest: addr("0x000000000000000000000000000000000000dEaD"),
        as_native: 0,
        token: addr("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
        denom: u256(1_000_000_000),
        relayer: addr("0x0000000000000000000000000000000000000B0b"),
        relayer_fee: u256(1_000_000),
    };
    fs::write("results/inputs/release.bin", words(&r)).unwrap();
    // The same release claiming another token or denomination than the burn leaf binds lands
    // on another root, which the vault never accepted.
    let b = bridge_guests::h(&[b"burn", &r.cfg, &r.token, &r.denom, &r.dest, &[r.as_native], &r.secret]);
    let honest = root_from_path(burn_leaf(&r.token, &r.denom, &b), r.index, &r.siblings);
    let weth = addr("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
    let forged = root_from_path(burn_leaf(&weth, &u256(10_000_000_000_000_000_000), &b), r.index, &r.siblings);
    assert_ne!(honest, forged);

    // Relay: synthetic LEZ blocks of 64 KiB and 1 MiB (LEZ's default block limit), 512-byte
    // records, 4 bridge burns each, appended to a tree that already holds 1,000,000 burns.
    let mut frontier = Frontier::empty();
    frontier.next = 1_000_000;
    frontier.left = (0..DEPTH as u64).map(|i| seeded("frontier", i)).collect();
    for (name, size) in [("relay_64k", 64 * 1024), ("relay_1m", 1024 * 1024)] {
        let mut block = vec![0u8; size];
        for (k, rec) in block.chunks_mut(512).enumerate() {
            rec[..32].copy_from_slice(&seeded("tx", k as u64));
        }
        for b in 0..4usize {
            let off = (b * size / 4) / 512 * 512;
            block[off..off + 8].copy_from_slice(b"BRGBURN1");
            block[off + 8..off + 40].copy_from_slice(&seeded("burn", b as u64));
        }
        // relay_block is the earlier one-block-per-proof measurement (no pairing or leaf binding);
        // the vault takes relay_range.
        let mut input = words(&(frontier.clone(), 4u32));
        input.extend_from_slice(&block);
        fs::write(format!("results/inputs/{name}.bin"), input).unwrap();
    }
    // Relay range (one batch = a contiguous chain of blocks after the vault's tip): 1 and 16
    // blocks of 64 KiB, and 1 block of 1 MiB, 4 burns per block.
    let z = bridge_guests::zeros();
    let mut check = frontier.clone();
    let r = check.append(seeded("check", 0), &z);
    assert_eq!(r, check.root(&z), "frontier root disagrees with append");
    let tip0 = seeded("lez-tip", 0);
    for (name, n, size) in [("range_1x64k", 1usize, 64 * 1024usize), ("range_16x64k", 16, 64 * 1024), ("range_1x1m", 1, 1024 * 1024)] {
        let mut tip = tip0;
        let mut bytes = Vec::new();
        let mut sizes = Vec::new();
        for k in 0..n {
            let mut block = vec![0u8; size];
            block[..32].copy_from_slice(&tip);
            for (t, rec) in block[32..].chunks_mut(512).enumerate() {
                rec[..32].copy_from_slice(&seeded("tx", (k * 100_000 + t) as u64));
            }
            for b in 0..4usize {
                let off = 32 + (b * (size - 32) / 4) / 512 * 512;
                // "BRGBURN1" ‖ program id ‖ token ‖ denomination ‖ B
                block[off..off + 8].copy_from_slice(b"BRGBURN1");
                block[off + 8..off + 40].copy_from_slice(&pairing.program_id);
                block[off + 40..off + 60].copy_from_slice(&addr("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"));
                block[off + 60..off + 92].copy_from_slice(&u256(1_000_000_000));
                block[off + 92..off + 124].copy_from_slice(&seeded("burn", (k * 4 + b) as u64));
            }
            let mut s = <sha2::Sha256 as sha2::Digest>::new();
            sha2::Digest::update(&mut s, b"/LEE/v0.3/Message/Block/\x00\x00\x00\x00\x00\x00\x00\x00");
            sha2::Digest::update(&mut s, &block);
            tip = sha2::Digest::finalize(s).into();
            sizes.push(size as u32);
            bytes.extend_from_slice(&block);
        }
        let mut input = words(&(pairing.clone(), frontier.clone(), tip0, 5_000_000u64, sizes));
        input.extend_from_slice(&bytes);
        fs::write(format!("results/inputs/{name}.bin"), input).unwrap();
    }
    // Failsafe relay: the same batches plus 4 claim records per block, a claim-nullifier tree
    // that already holds 1,000,000 claims, and counts for 5 tokens x 3 denominations.
    let usdc = addr("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
    let mut counts: Vec<([u8; 20], [u8; 32], u64, u64)> = Vec::new();
    for t in 0..5u64 {
        let tok: [u8; 20] = if t == 0 { usdc } else { seeded("token", t)[..20].try_into().unwrap() };
        for d in [1_000_000u128, 1_000_000_000, 10_000_000_000] {
            counts.push((tok, u256(d), 400_000, 300_000));
        }
    }
    let mut claim_frontier = Frontier::empty();
    claim_frontier.next = 1_000_000;
    claim_frontier.left = (0..DEPTH as u64).map(|i| seeded("claim-frontier", i)).collect();
    for (name, n, size) in [("failsafe_1x64k", 1usize, 64 * 1024usize), ("failsafe_16x64k", 16, 64 * 1024), ("failsafe_1x1m", 1, 1024 * 1024)] {
        let mut tip = tip0;
        let mut bytes = Vec::new();
        let mut sizes = Vec::new();
        for k in 0..n {
            let mut block = vec![0u8; size];
            block[..32].copy_from_slice(&tip);
            for (t, rec) in block[32..].chunks_mut(512).enumerate() {
                rec[..32].copy_from_slice(&seeded("tx", (k * 100_000 + t) as u64));
            }
            for b in 0..8usize {
                let off = 32 + (b * (size - 32) / 8) / 512 * 512;
                let tag: &[u8; 8] = if b % 2 == 0 { b"BRGBURN1" } else { b"BRGCLAIM" };
                block[off..off + 8].copy_from_slice(tag);
                block[off + 8..off + 40].copy_from_slice(&pairing.program_id);
                block[off + 40..off + 60].copy_from_slice(&usdc);
                block[off + 60..off + 92].copy_from_slice(&u256(1_000_000_000));
                block[off + 92..off + 124].copy_from_slice(&seeded("rec", (k * 8 + b) as u64));
            }
            let mut s = <sha2::Sha256 as sha2::Digest>::new();
            sha2::Digest::update(&mut s, b"/LEE/v0.3/Message/Block/\x00\x00\x00\x00\x00\x00\x00\x00");
            sha2::Digest::update(&mut s, &block);
            tip = sha2::Digest::finalize(s).into();
            sizes.push(size as u32);
            bytes.extend_from_slice(&block);
        }
        let mut input = words(&(pairing.clone(), frontier.clone(), claim_frontier.clone(), counts.clone(), tip0, 5_000_000u64, sizes));
        input.extend_from_slice(&bytes);
        fs::write(format!("results/inputs/{name}.bin"), input).unwrap();
    }
    println!("release image id 0x{}", hex::encode(id_bytes(RELEASE_ID)));
    println!("relay range id   0x{}", hex::encode(id_bytes(RELAY_RANGE_ID)));
    println!("relay image id   0x{}", hex::encode(id_bytes(RELAY_BLOCK_ID)));
    println!("wrote results/inputs/{{release,relay_64k,relay_1m}}.bin");
}

fn id_bytes(id: [u32; 8]) -> Vec<u8> {
    id.iter().flat_map(|w| w.to_le_bytes()).collect()
}

fn program(name: &str) -> (&'static [u8], [u32; 8]) {
    match name {
        "release" => (RELEASE_ELF, RELEASE_ID),
        "relay" => (RELAY_BLOCK_ELF, RELAY_BLOCK_ID),
        "range" => (RELAY_RANGE_ELF, RELAY_RANGE_ID),
        "failsafe" => (RELAY_FAILSAFE_ELF, RELAY_FAILSAFE_ID),
        _ => panic!("unknown program {name}"),
    }
}

fn env_for<'a>(input: &str, out: &'a mut Vec<u8>) -> ExecutorEnv<'a> {
    let bytes = fs::read(input).unwrap();
    ExecutorEnv::builder().write_slice(&bytes).stdout(out).build().unwrap()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(String::as_str) {
        Some("inputs") => write_inputs(),
        Some("execute") => {
            let (elf, _) = program(&a[2]);
            let mut out = Vec::new();
            let info = default_executor().execute(env_for(&a[3], &mut out), elf).expect("execute");
            let user: u64 = info.segments.iter().map(|s| s.cycles as u64).sum();
            let padded: u64 = info.segments.iter().map(|s| 1u64 << s.po2).sum();
            println!("EXEC {} {} user_cycles={user} padded_cycles={padded} segments={} guest_counters={} journal=0x{}", a[2], a[3], info.segments.len(), hex::encode(&out), hex::encode(&info.journal.bytes));
        }
        Some(cmd @ ("prove" | "groth16")) => {
            let (elf, id) = program(&a[2]);
            let opts = if cmd == "groth16" { ProverOpts::groth16() } else { ProverOpts::succinct() };
            let mut out = Vec::new();
            let t = Instant::now();
            let info = default_prover().prove_with_opts(env_for(&a[3], &mut out), elf, &opts).expect("prove");
            let secs = t.elapsed().as_secs_f64();
            info.receipt.verify(id).expect("verify");
            let size = bincode_len(&info.receipt);
            println!("PROVED {cmd} {} {} total_cycles={} user_cycles={} segments={} prove_s={secs:.1} receipt_bytes={size}", a[2], a[3], info.stats.total_cycles, info.stats.user_cycles, info.stats.segments);
            if cmd == "groth16" {
                let seal = risc0_ethereum_contracts::encode_seal(&info.receipt).expect("seal");
                fs::create_dir_all("../vault/proofs").unwrap();
                let j = serde_json::json!({
                    "seal": format!("0x{}", hex::encode(&seal)),
                    "journal": format!("0x{}", hex::encode(&info.receipt.journal.bytes)),
                    "image_id": format!("0x{}", hex::encode(id_bytes(id))),
                });
                fs::write(format!("../vault/proofs/{}.json", a[2]), serde_json::to_string_pretty(&j).unwrap()).unwrap();
                println!("wrote ../vault/proofs/{}.json (seal {} bytes)", a[2], seal.len());
            }
        }
        _ => eprintln!("usage: bridge-bench inputs | execute|prove|groth16 <release|relay|range> <input.bin>"),
    }
}

fn bincode_len(r: &Receipt) -> usize {
    risc0_zkvm::serde::to_vec(r).unwrap().len() * 4
}
