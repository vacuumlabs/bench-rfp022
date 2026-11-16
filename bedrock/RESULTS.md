# Bedrock Proof of Leadership inside RISC Zero: cost (2026-11-14)

Question: what would an Ethereum-side check of LEZ finality cost if it verified Bedrock's
consensus itself (a Bedrock light client proven in RISC Zero, Groth16-wrapped for Ethereum)?
The core per-block cost is verifying each block's Proof of Leadership (PoL).

## What PoL is

- Circuit: `mantle/pol.circom` in
  [logos-blockchain-circuits](https://github.com/logos-blockchain/logos-blockchain-circuits/tree/16234c98d279a32084a17e59033fa873429f9d8a),
  Circom, Groth16 over BN254 (`bn128`). Verifying key from the v0.8.0-rc.1 release
  (`fixtures/pol_verification_key_v0.8.0-rc.1.json`): **9 public signals** (slot, epoch nonce,
  lottery thresholds t0/t1, aged and latest ledger roots, the leader key in two parts, and the
  entropy contribution). v0.7.3's key had 10.
- One PoL per Bedrock block.
- Testnet Cryptarchia (genesis template, logos-blockchain at `468ebff`): slot 1 s, active-slot
  coefficient 1/30 (about one block every 30 s, about 2,880 a day), security parameter k = 120
  blocks. LEZ's finality depth (PR 1026): 180 Bedrock blocks.

## Measured (executor, RISC Zero 3.1.0, accelerated BN254: `substrate-bn` v0.6.0-risczero.0)

`methods/guest`: snarkjs-compatible Groth16 verification. Two inputs: `synthetic` (a valid
Groth16 tuple for a 10-input BN254 key, built from a known trapdoor; accepted) and `pol_vk`
(Bedrock's real PoL key, every point checked on its curve, with that tuple; rejected, same work).

| Variant | Input | Point checks | vk_x (MSM) | Pairings | Total, user cycles (segments) |
|---|---|---|---|---|---|
| Generic (key points validated, e(α,β) computed) | real PoL key, 9 inputs | 20.44M | 11.72M | 21.90M (4 pairings) | 54.26M (55) |
| Generic | synthetic, 10 inputs, accepted | 20.44M | 13.02M | 21.91M | 55.57M (57) |
| **Deployed form** (key a constant, e(α,β) precomputed) | **real PoL key, 9 inputs** | 5.11M (proof only) | 11.72M | 18.71M (3 pairings) | **47.81M (49)**, of which **35.5M** per-proof work |
| Deployed form | synthetic, 10 inputs, accepted | 5.11M | 13.02M | 18.71M | 49.12M (50) |

The "total" of the deployed form includes about 12M of one-off setup in this harness (key
decoding and e(α,β), outside the counters); a batch verifier pays it once. So: **about 35.5M
cycles per PoL proof**, at the desktop's measured ~108 s per 2^20 segment about **an hour of
proving per Bedrock block**.

**Real proof** (deployed form, synthetic accepted input, desktop, CPU, `RISC0_DEV_MODE=0`):
50 segments, 52.4M total cycles, **5,425 s (1 h 30 min) wall clock**, 108.5 s per segment, peak
memory 9.2 GiB (`results/logs/pol_fixed_prove.log`).

## What it implies for a trustless `isFinalLezBlock`

- **Per relay batch from a checkpoint:** finality to 180 Bedrock blocks needs at least 180 PoL
  checks: about 6.4 billion cycles, about 6,100 segments, about 7.6 days of our desktop's proving per
  batch.
- **Following Bedrock continuously** (each block proven once, recursively): about 2,880 blocks
  a day × 35.5M ≈ **102 billion cycles a day**, about 1.2M cycles per second sustained. Our
  desktop proves about 10k cycles per second, so roughly 110 such machines running all the
  time, or a GPU proving service.
- **Verifying PoL natively on Ethereum instead:** a 9-input Groth16 check costs about 240k gas
  (EIP-1108 prices: 4 pairings ≈ 181k, 9 scalar multiplications ≈ 55k), so 180 blocks ≈ 43M gas,
  about a whole Ethereum block, per relay batch. Estimate, not measured.
- **Not counted in any of these:** the rest of a Bedrock light client. PoL's public inputs
  include the aged and latest ledger roots and the epoch nonce, so the verifier must also track
  Bedrock's stake snapshots and nonce evolution (and Cryptarchia's fork choice with orphan
  proofs), and check the LEZ channel inscription and its accredited signatures. No published
  Bedrock light client exists to reuse.

## Not done

A real PoL proof from Bedrock's own prover: generating its witness needs linking the release's
prebuilt circuit library, which this session's auto-mode permission check denied. The cost does
not depend on it: Groth16 verification work is set by the curve and the public-input count.
