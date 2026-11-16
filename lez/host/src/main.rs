//! Inbound claim proof on RISC Zero 3.0.5 (LEZ's version).
//!   claim-bench inputs <receipts.json> <headers.json> <distance>
//!       one real mainnet block's 272 receipts, with tx 250 replaced by a vault Deposit
//!       receipt, placed in the block <distance> below the last of the real mainnet headers
//!       (the recorded header). That block's receipts root is replaced, and it and every
//!       header above it re-hashed, so sizes and fields stay real. Writes
//!       results/inputs/claim_d<distance>.bin
//!   claim-bench execute <input>         cycles
//!   claim-bench prove <input>           succinct STARK, timed
use std::{fs, time::Instant};

use alloy::{
    consensus::{Header, Receipt, ReceiptEnvelope, TxReceipt},
    eips::eip2718::Encodable2718,
    rpc::types::{Header as RpcHeader, TransactionReceipt},
};
use alloy_primitives::{keccak256, Address, Log, LogData, B256};
use alloy_trie::{proof::ProofRetainer, HashBuilder, Nibbles};
use claim_methods::{CLAIM_ELF, CLAIM_ID};
use risc0_zkvm::{default_executor, default_prover, ExecutorEnv, ProverOpts};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// The pairing `cfg` commits to (same layout as BridgeVault and bench/zk's `Pairing`).
#[derive(Serialize)]
struct Pairing {
    evm_chain_id: u64,
    vault: [u8; 20],
    lez_chain_id: [u8; 32],
    zone_id: [u8; 32],
    program_id: [u8; 32],
    rfp021_instance: [u8; 32],
}

#[derive(Serialize)]
struct Input {
    pairing: Pairing,
    dest: [u8; 32],
    header_sizes: Vec<u32>,
    depth: u64,
    leaf_index: u64,
    siblings: Vec<[u8; 32]>,
    tx_index: u64,
    receipt: Vec<u8>,
    proof: Vec<Vec<u8>>,
    log_index: u32,
    secret: [u8; 32],
    relayer: [u8; 32],
    relayer_fee: [u8; 32],
    protocol_fee: [u8; 32],
}

fn h(parts: &[&[u8]]) -> [u8; 32] {
    let mut s = Sha256::new();
    for p in parts {
        s.update(p);
    }
    s.finalize().into()
}

fn seeded(tag: &str, i: u64) -> [u8; 32] {
    h(&[tag.as_bytes(), &i.to_be_bytes()])
}

fn word(v: u128) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[16..].copy_from_slice(&v.to_be_bytes());
    w
}

fn inputs(receipts_json: &str, headers_json: &str, distance: usize) {
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(receipts_json).unwrap()).unwrap();
    let receipts: Vec<TransactionReceipt> = serde_json::from_value(v["result"].clone()).unwrap();
    let mut encoded: Vec<Vec<u8>> = receipts.iter().map(|r| r.clone().into_primitives_receipt().inner.encoded_2718()).collect();

    // A vault Deposit receipt in place of tx 250: Deposit(token, denom, C, index).
    let vault: Address = "0x1111111111111111111111111111111111111111".parse().unwrap();
    let pairing = Pairing {
        evm_chain_id: 1,
        vault: vault.0 .0,
        lez_chain_id: h(&[b"lez"]),
        zone_id: h(&[b"zone"]),
        program_id: h(&[b"program"]),
        rfp021_instance: h(&[b"rfp021"]),
    };
    let cfg = h(&[b"cfg", &pairing.evm_chain_id.to_be_bytes(), &pairing.vault, &pairing.lez_chain_id, &pairing.zone_id, &pairing.program_id, &pairing.rfp021_instance]);
    let dest = seeded("lez-destination-key", 0);
    let token: Address = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".parse().unwrap();
    let denom = word(1_000_000_000);
    let secret = seeded("deposit-secret", 0);
    let commitment = h(&[b"dep", &cfg, token.as_slice(), &denom, &dest, &secret]);
    let mut token_topic = [0u8; 32];
    token_topic[12..].copy_from_slice(token.as_slice());
    let mut data = commitment.to_vec();
    data.extend_from_slice(&word(4242));
    let log = Log {
        address: vault,
        data: LogData::new_unchecked(vec![keccak256("Deposit(address,uint256,bytes32,uint256)"), B256::from(token_topic), B256::from(denom)], data.into()),
    };
    let tx_index = 250usize;
    let cumulative = receipts[tx_index].inner.cumulative_gas_used();
    let receipt = Receipt { status: true.into(), cumulative_gas_used: cumulative, logs: vec![log] };
    encoded[tx_index] = ReceiptEnvelope::Eip1559(receipt.with_bloom()).encoded_2718();

    let target = Nibbles::unpack(alloy_rlp::encode(tx_index as u64));
    let mut items: Vec<(Nibbles, &Vec<u8>)> = encoded.iter().enumerate().map(|(i, e)| (Nibbles::unpack(alloy_rlp::encode(i as u64)), e)).collect();
    items.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hb = HashBuilder::default().with_proof_retainer(ProofRetainer::new(vec![target.clone()]));
    for (k, e) in &items {
        hb.add_leaf(k.clone(), e);
    }
    let root = hb.root();
    let proof: Vec<Vec<u8>> = hb.take_proof_nodes().matching_nodes_sorted(&target).into_iter().map(|(_, b)| b.to_vec()).collect();

    // Real mainnet headers; check our encoding reproduces every hash and parent link.
    let hv: serde_json::Value = serde_json::from_str(&fs::read_to_string(headers_json).unwrap()).unwrap();
    let rpc: Vec<RpcHeader> = serde_json::from_value(hv["headers"].clone()).unwrap();
    let mut real: Vec<Header> = Vec::new();
    for r in &rpc {
        assert_eq!(keccak256(alloy_rlp::encode(&r.inner)), r.hash, "header encoding");
        if let Some(p) = real.last() {
            assert_eq!(r.inner.parent_hash, keccak256(alloy_rlp::encode(p)));
        }
        real.push(r.inner.clone());
    }
    let n = real.len();
    assert!(distance < n);
    // Deposit block = recorded - distance, with our receipts root; re-hash upwards.
    let mut chain: Vec<Header> = real[n - 1 - distance..].to_vec();
    chain[0].receipts_root = root;
    for j in 1..chain.len() {
        chain[j].parent_hash = keccak256(alloy_rlp::encode(&chain[j - 1]));
    }
    let headers: Vec<Vec<u8>> = chain.iter().rev().map(|h| alloy_rlp::encode(h)).collect();
    let avg = headers.iter().map(Vec::len).sum::<usize>() / headers.len();

    let input = Input {
        pairing,
        dest,
        header_sizes: headers.iter().map(|h| h.len() as u32).collect(),
        depth: distance.min(64) as u64,
        leaf_index: 1_000_000,
        siblings: (0..32).map(|i| seeded("acc-sibling", i)).collect(),
        tx_index: tx_index as u64,
        receipt: encoded[tx_index].clone(),
        proof,
        log_index: 0,
        secret,
        relayer: seeded("relayer", 0),
        relayer_fee: word(1_000_000),
        protocol_fee: word(0),
    };
    fs::create_dir_all("results/inputs").unwrap();
    // Serialised input, then the headers as raw bytes (read by the guest from stdin).
    let mut words: Vec<u8> = risc0_zkvm::serde::to_vec(&input).unwrap().iter().flat_map(|w| w.to_le_bytes()).collect();
    words.extend(headers.concat());
    let out = format!("results/inputs/claim_d{distance}.bin");
    fs::write(&out, words).unwrap();
    println!("claim image id 0x{}", hex::encode(CLAIM_ID.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>()));
    println!("receipts: {} (tx 250 replaced by a Deposit receipt), proof nodes {}, headers {} (avg {avg} B, real chain of {n} checked), wrote {out}", encoded.len(), input.proof.len(), headers.len());
}

fn env_for<'a>(input: &str, out: &'a mut Vec<u8>) -> ExecutorEnv<'a> {
    ExecutorEnv::builder().write_slice(&fs::read(input).unwrap()).stdout(out).build().unwrap()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(String::as_str) {
        Some("inputs") => inputs(&a[2], &a[3], a[4].parse().unwrap()),
        Some("execute") => {
            let mut out = Vec::new();
            let info = default_executor().execute(env_for(&a[2], &mut out), CLAIM_ELF).expect("execute");
            let user: u64 = info.segments.iter().map(|s| s.cycles as u64).sum();
            let padded: u64 = info.segments.iter().map(|s| 1u64 << s.po2).sum();
            let (chain, acc, incl, rest): (u64, u64, u64, u64) = risc0_zkvm::serde::from_slice(&out.chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect::<Vec<u32>>()).unwrap();
            println!("EXEC claim user_cycles={user} padded_cycles={padded} segments={} header_chain={chain} accumulator={acc} receipt_inclusion={incl} open_and_nullifier={rest}", info.segments.len());
        }
        Some("prove") => {
            let mut out = Vec::new();
            let t = Instant::now();
            let info = default_prover().prove_with_opts(env_for(&a[2], &mut out), CLAIM_ELF, &ProverOpts::succinct()).expect("prove");
            let secs = t.elapsed().as_secs_f64();
            info.receipt.verify(CLAIM_ID).expect("verify");
            println!("PROVED claim {} total_cycles={} user_cycles={} segments={} prove_s={secs:.1}", a[2], info.stats.total_cycles, info.stats.user_cycles, info.stats.segments);
        }
        _ => eprintln!("usage: claim-bench inputs <receipts.json> | execute <input> | prove <input>"),
    }
}
