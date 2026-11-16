//! Inbound claim (user, on LEZ): the vault's Deposit log is in a receipt under the deposit
//! block's header; that header is linked by parent hashes to a header the RFP-021 instance
//! recorded, which is a leaf of the header accumulator, of which only the root R is public;
//! the log comes from the vault named by the pairing that `cfg` commits to; the log's commitment C opens with the user's secret; the
//! nullifier N = H(cfg, secret, statement id) keeps the statement id private.
#![no_main]
use alloy_consensus::{Header, ReceiptEnvelope, TxReceipt};
use alloy_eips::eip2718::Decodable2718;
use alloy_primitives::{keccak256, Bytes, B256};
use alloy_rlp::Decodable;
use alloy_trie::{proof::verify_proof, Nibbles};
use risc0_zkvm::guest::env;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

risc0_zkvm::guest::entry!(main);

/// The pairing `cfg` commits to (same layout as BridgeVault and bench/zk's `Pairing`).
#[derive(Serialize, Deserialize)]
struct Pairing {
    evm_chain_id: u64,
    vault: [u8; 20],
    lez_chain_id: [u8; 32],
    zone_id: [u8; 32],
    program_id: [u8; 32],
    rfp021_instance: [u8; 32],
}

#[derive(Deserialize)]
struct Input {
    // cfg is recomputed from the pairing, and the log must come from its vault
    pairing: Pairing,
    // the LEZ destination the deposit committed to (a seed-derived key of a private account,
    // or a public account the user confirmed); the program mints only to it (Sec.3)
    dest: [u8; 32],
    // sizes of the RLP headers that follow the serialised input as raw bytes, from the
    // recorded header (first) down to the deposit block (last)
    header_sizes: Vec<u32>,
    // public: minimum blocks between the deposit block and the recorded header (G6)
    depth: u64,
    // header accumulator leaf and its path (append-only tree, depth 32)
    leaf_index: u64,
    siblings: Vec<[u8; 32]>,
    // receipt inclusion
    tx_index: u64,
    receipt: Vec<u8>,
    proof: Vec<Vec<u8>>,
    log_index: u32,
    // opening
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

pub fn main() {
    let i: Input = env::read();
    let mut all = Vec::new();
    std::io::Read::read_to_end(&mut env::stdin(), &mut all).unwrap();
    let mut headers: Vec<&[u8]> = Vec::with_capacity(i.header_sizes.len());
    let mut off = 0usize;
    for &n in &i.header_sizes {
        headers.push(&all[off..off + n as usize]);
        off += n as usize;
    }
    assert_eq!(off, all.len());
    let c = env::cycle_count;
    let t0 = c();
    let p = &i.pairing;
    let cfg = h(&[b"cfg", &p.evm_chain_id.to_be_bytes(), &p.vault, &p.lez_chain_id, &p.zone_id, &p.program_id, &p.rfp021_instance]);
    assert_eq!(i.siblings.len(), 32, "accumulator path length");

    // 1. Parent-hash chain from the recorded header down to the deposit block. Only the two
    //    ends are decoded; each link reads the first field (parentHash) of the next header.
    let recorded_hash = keccak256(headers[0]);
    let mut expect = recorded_hash;
    for (j, raw) in headers.iter().enumerate() {
        if j > 0 {
            assert_eq!(keccak256(raw), expect, "parent hash chain broken");
        }
        let mut b: &[u8] = raw;
        let list = alloy_rlp::Header::decode(&mut b).expect("header rlp");
        assert!(list.list);
        expect = B256::decode(&mut b).expect("parent hash");
    }
    let recorded = Header::decode(&mut &headers[0][..]).expect("recorded header");
    let deposit = Header::decode(&mut &headers[headers.len() - 1][..]).expect("deposit header");
    assert_eq!(recorded.number - deposit.number, (headers.len() - 1) as u64);
    assert!(deposit.number + i.depth <= recorded.number, "deposit not deep enough");
    let block_number = deposit.number;
    let receipts_root = deposit.receipts_root.0;
    let tc = c();

    // 2. Recorded header's leaf in the accumulator; only R leaves the proof.
    let mut cur = h(&[b"hdr", recorded_hash.as_slice()]);
    let mut idx = i.leaf_index;
    for s in &i.siblings {
        cur = if idx & 1 == 0 { h(&[&cur, s]) } else { h(&[s, &cur]) };
        idx >>= 1;
    }
    let accumulator_root = cur;
    let t1 = c();

    // 3. Receipt under the deposit block's receipts root.
    let proof: Vec<Bytes> = i.proof.iter().cloned().map(Bytes::from).collect();
    let key = Nibbles::unpack(alloy_rlp::encode(i.tx_index));
    verify_proof(B256::from(receipts_root), key, Some(i.receipt.clone()), proof.iter()).expect("receipt not under header");
    let r = ReceiptEnvelope::decode_2718(&mut i.receipt.as_slice()).expect("receipt decode");
    assert!(r.status());
    let t2 = c();

    // 4. The vault's Deposit(token indexed, denom indexed, commitment, index) log.
    let log = &r.logs()[i.log_index as usize];
    assert_eq!(log.address.0 .0, p.vault, "log not from this pairing's vault");
    let topics = log.topics();
    assert_eq!(topics[0], keccak256("Deposit(address,uint256,bytes32,uint256)"));
    let token: [u8; 32] = topics[1].0;
    let denom: [u8; 32] = topics[2].0;
    let commitment: [u8; 32] = log.data.data[..32].try_into().unwrap();
    assert_eq!(commitment, h(&[b"dep", &cfg, &token[12..], &denom, &i.dest, &i.secret]), "commitment does not open");

    // 5. Statement id (deterministic from the attested log) stays private; nullifier is public.
    let mut fact = Vec::with_capacity(64 + log.data.data.len());
    fact.extend_from_slice(&block_number.to_be_bytes());
    fact.extend_from_slice(&i.tx_index.to_be_bytes());
    fact.extend_from_slice(&i.log_index.to_be_bytes());
    fact.extend_from_slice(log.address.as_slice());
    for t in topics {
        fact.extend_from_slice(t.as_slice());
    }
    fact.extend_from_slice(&log.data.data);
    let stmt_id = keccak256(&fact);
    let nullifier = h(&[b"nul", &cfg, &i.secret, stmt_id.as_slice()]);
    let t3 = c();

    env::commit(&(cfg, accumulator_root, i.depth, nullifier, token, denom, i.dest, i.protocol_fee, i.relayer, i.relayer_fee));
    env::write(&(tc - t0, t1 - tc, t2 - t1, t3 - t2));
}
