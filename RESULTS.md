# RFP-022 bench: results

Prototype measurements for the design in the Slipgate proposal (RFP-022 with RFP-021). Not audited code. Overview and gaps: [`README.md`](README.md).

| Folder | What | RISC Zero |
|---|---|---|
| `lez/` | Inbound claim proof (the user proves on their device, on LEZ) | 3.0.5, LEZ v0.4.0-rc3's version and lock |
| `zk/` | Release proof (user, outbound) and the LEZ burn relay's per-block work | 3.1.0, the version the fixed on-chain verifier accepts |
| `vault/` | Solidity vault prototype, Foundry tests, real-transaction gas script | risc0-ethereum v3.1.0 contracts |
| `run_proofs.sh` | Queued real proving + Groth16 + real-proof vault test | |

Relay guests: `relay_block` (one block, used for the proving numbers; predates the pairing and leaf binding) and `relay_range`
(the batch the vault expects: a contiguous chain of blocks after the vault's tip).

## Review-1 fixes (2026-11-14)

An adversarial review found soundness holes in the prototypes; fixed and re-measured:
- **Burn leaf binds what was burned.** A burn record is `"BRGBURN1" ‖ program id ‖ token ‖
  denomination ‖ B`; `relay_range` counts it only if the program id is this pairing's bridge
  program and appends the leaf `H("leaf", token, denom, B)`. `release` recomputes that leaf,
  so a `B` that opens to another token or amount than was burned lands on a root the vault
  never accepted (checked in the host). The release nullifier is `H("bnul", cfg, s, B)`.
- **`cfg` is opened, not trusted.** Every guest takes the pairing (chain id, vault, LEZ chain
  id, zone id, program id, RFP-021 instance) and recomputes `cfg`; the claim takes the vault
  address from it, so a log from any other contract is refused. The vault computes the same
  `cfg` from `block.chainid` and `address(this)` in its constructor and refuses every proof if
  `block.chainid` changes (chain split).
- **Deposit commits to its LEZ destination** (Sec.3): `C = H("dep", cfg, token, denom, dest,
  s)`, and the claim journal carries `dest`, so whoever submits the claim cannot redirect it.
- **Fees vs denominations.** The inbound fee is charged on top at deposit (`claimFee`, at most
  1%), so the claim mints the full denomination and a user can always burn exactly what they
  minted. The outbound relayer fee is still taken at release.
- **Fee changes don't break proofs.** `setRelayerFee` keeps the previous fee valid for 7 days.
- **Commitments keyed per (token, denomination, C)**, so nobody can block a deposit by
  front-running its `C` under another token.
- Claim checks the accumulator path is 32 long. Tests: `test_review1_fixes`, plus the existing
  rejects and the real-proof test, all pass.

## Ethereum gas (measured 2026-11-14, after the review-1 fixes)

Fork block 26,414,052 (the public RPC no longer serves the 11-13 block without an archive
token). Before the fixes, at block 26,402,536: deposits 104,605 / 104,357 / 155,916 / 84,958;
releases 105,387 / 144,398 / 127,882 / 151,026; relay 73,928 / 90,644.

Real transactions on an anvil fork of Ethereum mainnet, with the real
USDC, USDT and WETH contracts (`vault/scripts/gas_anvil.sh`, raw: `vault/results/gas.csv`).
These exclude proof verification (an accept-all verifier stands in) and use a mock for
`isFinalLezBlock`.

| Operation | Gas used |
|---|---|
| Deposit USDC, typical (fee on top) | 107,195 |
| Deposit USDT, typical | 106,959 |
| Deposit USDC, first ever into the vault | 158,506 |
| Deposit ETH (wrapped to WETH) | 87,598 |
| Release USDC, recipients already hold USDC | 110,575 |
| Release USDC, first release, recipients new to USDC | 144,786 |
| Release USDT, recipients new to USDT | 128,294 |
| Release as native ETH | 151,462 |
| Relay batch, steady state, **any number of LEZ blocks** | **74,063** |
| Relay batch, first one | 90,779 |

RISC Zero's real `RiscZeroGroth16Verifier` 3.1.0, verifying RISC Zero's official 3.1 test
receipt: **224,813 gas** of execution (`vault/test/BridgeVault.t.sol`, no fork needed); a
tampered journal is rejected. With a real proof of our release guest the whole release is
295,479 execution gas, about 324,100 as a transaction (below; 295,091 / 323,700 before the
fixes). The verifier's share of a real release is about 185k of execution (295,479 −
110,575 for the same release without it). A relay batch is therefore about 265,000–
300,000 gas whatever its length (no Groth16 relay proof submitted yet).

**Relay change (2026-11-13).** The first prototype stored one slot per relayed LEZ block
hash (96,432 gas for 1 block, 421,986 for 16, about 22k per block). A batch now covers the
contiguous range after the vault's last relayed block (`lezTip`): the guest proves each
block's previous hash is the block before, so no block can be skipped or relayed twice,
and the vault stores only the new root, tip and height, after checking `isFinalLezBlock`
of the new tip. Releases also check the relayer fee equals the fixed configured fee per
(token, denomination) (`setRelayerFee`), which costs about 2.2k gas more per release than
the first prototype's 1% ceiling check. Earlier figures (fork block 26,401,801):
deposits 104,646 / 104,398 / 155,957 / 84,958; releases 103,198 / 142,209 / 125,693 / 148,837.

## zkVM cycles (measured 2026-11-12, executor, no proving)

| Program | Work cycles (in-guest counters) | Whole run, user (padded) |
|---|---|---|
| Claim: parent-hash chain, 1 header (deposit block is the recorded one) | 22,196 | 594,640 (1,048,576) |
| Claim: parent-hash chain, 32 real mainnet headers | 359,100 | 977,403 (1,179,648) |
| Claim: parent-hash chain, 300 real mainnet headers (about an hour) | 3,278,015 | 4,221,414 (5,242,880) |
| Claim: accumulator membership, depth 32 | 43,284 | |
| Claim: receipt inclusion (272-receipt block, 5 proof nodes) | 65,757 | |
| Claim: open commitment, statement id, nullifier | 10,470 | |
| **Claim total**, before the header chain (2026-11-13) | **119,511** | 573,227 (1,048,576, one segment) |
| **Claim total after review-1** (cfg opened, destination committed), deposit in the recorded block | **144,489** | 620,370 (1,048,576); 1,003,195 (1,179,648) at 32 headers; 4,247,745 (5,242,880) at 300 |
| **Release** (burn-tree membership depth 32, opening, nullifier, ABI journal) | **46,980** (48,320 after review-1: leaf binding, nullifier over B) | 225,767 (262,144, one segment); 227,107 after |
| Relay, 64 KiB LEZ block: block hash | 72,811 | |
| Relay, 64 KiB LEZ block: scan + 4 burn appends | 219,733 | 576,954 (1,048,576) |
| Relay, 1 MiB LEZ block: block hash | 1,143,913 | |
| Relay, 1 MiB LEZ block: scan + 4 burn appends | 281,173 | 3,951,028 (8,388,608) |
| **Relay range** (`relay_range`, one batch), 1 × 64 KiB block, 4 burns | 400,353 after review-1 (hash 73,069, scan + leaf + appends 195,354); 377,691 before | 733,850 (1,048,576) |
| Relay range, 16 × 64 KiB blocks, 64 burns | 4,421,922 after (hash 1,169,104, scan + leaf + appends 3,119,136); 4,138,989 before | 6,998,738 (11,534,336) |
| Relay range, 1 × 1 MiB block, 4 burns | 1,536,735 after (hash 1,144,171, scan + leaf + appends 260,634); 1,514,073 before | 4,111,796 (8,388,608) |

"Whole run" includes reading the input, which these harnesses do with RISC Zero's word
serialisation; the work cycles are what the protocol itself costs.

**Claim header chain (G15, added 2026-11-14).** An RFP-021 update records one finalized
header, not every block, so the claim proves the parent-hash chain from the deposit block up
to a recorded header (private input; the recorded header's hash is the accumulator leaf). Each
link is a Keccak of the next header (about 647 bytes) and a read of its first field; only
the two ends are fully decoded. Cost: about 10.9k cycles per header, linear. Data: 300
consecutive real mainnet headers ending at block 26,398,402 (the finalized block of our
RFP-021 fixture), fetched with `eth_getBlockByNumber` (`lez/fixtures/headers.json`); the host
checks that its encoding reproduces every real hash and link. The deposit block's receipts
root is replaced by the synthetic receipts' root and every header above it re-hashed, so
sizes and fields stay real. A changed byte in any header is rejected. The headers are read
as raw bytes (not word serialisation), as a client would pass them. The claim uses a real
Ethereum mainnet block's 272 receipts (block 26,398,402) with transaction 250's receipt
replaced by a vault `Deposit` receipt, so the trie shape and proof depth are real. The
relay block is synthetic but sized like a real LEZ block (whose hash is SHA-256 over the
whole borsh-encoded block, `lez/common/src/block.rs`).

`relay_range` is the batch guest the vault now expects: work = block hashing (about 1.14M
cycles per MiB) + about 44k cycles per burn appended to the depth-32 tree + about 125k fixed
(the tree root before and after, the ABI journal). Executor only; proving follows the
segment count at about 100–110 s per 2^20 segment (the `relay_block` proofs below), so the
16-block batch would take about 18 minutes on this desktop.

## Real proving and Groth16 (measured 2026-11-13)

`run_proofs.sh`: `RISC0_DEV_MODE=0`, CPU prover, AMD Ryzen 7 9800X3D (8 cores / 16 threads,
15 GiB under WSL2). Each proof in its own process; peak memory = that process's maximum RSS.
Raw: `results/proving.csv`, logs in `results/logs/`.

| Proof | RISC Zero | Padded cycles (segments) | Proving | Peak memory | Receipt |
|---|---|---|---|---|---|
| Claim (user, LEZ), no header chain | 3.0.5 | 1,048,576 (1) | **158 s** | 9.1 GiB | |
| Claim, 32-header chain (measured 2026-11-14) | 3.0.5 | 1,179,648 (2) | 177 s | 9.2 GiB | |
| Claim, 300-header chain (measured 2026-11-14) | 3.0.5 | 5,242,880 (5) | 674 s | 9.2 GiB | |
| Release (user, outbound), succinct STARK | 3.1.0 | 262,144 (1) | **32 s** | 2.3 GiB | 225 KB |
| Relay, one 64 KiB LEZ block | 3.1.0 | 1,048,576 (1) | 104 s | 9.1 GiB | 226 KB |
| Relay, one 1 MiB LEZ block | 3.1.0 | 8,388,608 (8) | 882 s | 9.2 GiB | 226 KB |
| Release, Groth16 for Ethereum (Docker) | 3.1.0 | 262,144 (1) | **71 s** end to end | 2.4 GiB host process, then 7.45 GiB in the container | 260-byte seal |
| Release after review-1, Groth16 (2026-11-14) | 3.1.0 | 262,144 (1) | 74 s end to end | 2.3 GiB host process | 260-byte seal |

- The claim pads to 2^20 because the harness reads its input with RISC Zero's word
  serialisation (573k user cycles for 119.5k cycles of work); one segment costs about 9 GiB
  whatever is in it. A one-segment 2^19 proof (the RFP-021 inclusion bench) took 91 s / 4.6 GiB.
- Groth16 is the whole `ProverOpts::groth16()` call: the succinct STARK, the identity_p254
  recursion and the Docker stark-to-snark step: 71.5 s and 70.6 s in two runs. The script's
  first run took 290 s because it pulled the 5.4 GB `risczero/risc0-groth16-prover` image
  (tagged locally at 00:38:12 during that run). Container memory is the peak of streamed
  `docker stats` samples (`results/logs/groth16_docker_mem.log`), not a cgroup maximum.
- Relay: proving time follows the segment count (about 100–110 s per 2^20 segment), so a
  batch's cost is roughly linear in the bytes of LEZ blocks it covers.

**Release with a real proof, on a mainnet fork** (`vault/test/RealProof.t.sol`, real USDC,
the real `RiscZeroGroth16Verifier` 3.1.0 with its 3.1 control root): **294,902 execution
gas, about 323,500 gas as a transaction** (295,091 / 323,687 after the relay change) (21,000 + calldata + execution; `dest` already
holds USDC on the fork). The same proof with one byte of
`dest` changed is rejected by the verifier, and the valid proof replayed is rejected by the
nullifier. The script's first run failed on the test's own balance assertion (it assumed
`dEaD` held no USDC); the release itself had succeeded. Fixed and re-run
(`results/logs/vault_real_proof_rerun.log`).

## Failsafe relay (2026-11-15, executor, RISC Zero 3.1.0)

`relay_failsafe` is `relay_range` plus what the failsafe (proposal §9) needs from
the relay: each claim's nullifier appended to a claim-nullifier tree (1,000,000 entries before),
and cumulative claim and burn counts per (token, denomination) (5 tokens × 3 denominations)
committed as one hash. Blocks carry 4 burns and 4 claims each. `relay_range` unchanged; the
vault still takes `relay_range`'s journal (the failsafe journal is 12 words instead of 7).

| Batch | `relay_range` work (segments) | `relay_failsafe` work (segments) | Extra |
|---|---|---|---|
| 1 × 64 KiB, 4 burns (+4 claims) | 400,353 (1) | 684,696 (2) | +284k |
| 16 × 64 KiB, 64 burns (+64 claims) | 4,421,922 (11) | 7,372,177 (15) | +2.95M |
| 1 × 1 MiB, 4 burns (+4 claims) | 1,536,735 (8) | 1,798,066 (9) | +261k |

So a claim costs the relay about 44k cycles, like a burn, plus about 105k fixed per batch (two
more roots and the counts hash). Proving cost rises in proportion to claims relayed; at ~108 s
per segment, the 16-block batch goes from about 20 to about 27 minutes on the desktop.
Vault gas for the larger journal and two more stored words: not measured, estimated at about
+10k per batch.
