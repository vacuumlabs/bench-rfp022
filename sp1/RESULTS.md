# SP1 second release path: execution cost (2026-11-15)

Purpose: size the second, independent release path (proposal §9) on SP1 before
filing. This is a **port** of bench/zk's release and relay statements for measurement only.
The delivered second path is written independently of the RISC Zero code; reusing this port
would defeat the point.

Setup:
- SP1 6.10.0 (`cargo-prove` d83ed91), executor only, no proving.
- SHA-256 through SP1's precompile: `sha2` patched to
  `sp1-patches/RustCrypto-hashes` tag `patch-sha2-0.10.9-sp1-6.10.0`.
- Inputs: the same as bench/zk. The release uses the pairing and input of
  `vault/proofs/release.json`; the relay uses the same synthetic LEZ blocks as the
  `range_*` inputs.
- Journals compared byte for byte with RISC Zero's: the release against
  `vault/proofs/release.json`; the relay against `r0_relay_journals.txt` (produced by
  `bridge-bench execute range …`, which now prints the journal).

| Program | SP1 instructions | RISC Zero 3.1 user cycles | Journal identical |
|---|---|---|---|
| Release | 97,538 (366,519 without the SHA-256 precompile) | 48.3k (work) | yes |
| Relay, 1 × 64 KiB block, 4 burns | 649,517 | 733,850 | yes |
| Relay, 16 × 64 KiB blocks, 64 burns | 7,465,841 | 6,998,738 | yes |
| Relay, 1 × 1 MiB block, 4 burns | 4,184,237 | 4,111,796 | yes |

Reading: SP1 instructions and RISC Zero cycles are different units, and proving cost per unit
differs, so this sizes the work but not the proving time. The second relay's work is about the
same as the first's, which supports "about double relay proving".

## Verifier gas (measured 2026-11-16)

`contracts/test/SP1OfficialGas.t.sol` (`forge test -vv` in `contracts/`, sp1-contracts at
d3629729 in `contracts/lib/`):

| Verifier | Proof | Execution gas |
|---|---|---|
| `SP1VerifierGroth16` v6.0.0 | SP1's official v6 test proof (96-byte public values, 356-byte proof) | **219,861** |
| `RiscZeroGroth16Verifier` 3.1.0 (vault test, for comparison) | RISC Zero's 3.1 test receipt | 224,813 |

- v6.1.0, the version the SDK's circuits and the proposal pin, differs from v6.0.0 only in
  constants (VERSION, VERIFIER_HASH, VK_ROOT, verifying key) and the order of two checks, so its
  gas is the same.
- Our release journal is 288 bytes instead of 96: SHA-256 of it costs 614 gas, a few hundred
  more than the test proof's.
- The calldata of the 356-byte proof is at most about 5.7k gas.
- Hence **about 226k more gas per release after an outbound sunset**, replacing the earlier
  estimate of about 300k.
- A second-relay batch costs the same verifier gas whatever its length, so before a sunset the
  second relay posts daily (about 220k a day), not with every first-relay batch.

`contracts/test/SP1ReleaseGas.t.sol` does the same on our own release proof, once
`contracts/fixtures/release-groth16.json` exists (see below).

## Real proving: did not fit this machine (2026-11-16)

`release-script prove-release` (core → compressed → shrink → wrap → Groth16 through the
`ghcr.io/succinctlabs/sp1-gnark:v6.1.0` Docker image), desktop, WSL with 15.2 GB:

| Run | What happened | Wall clock | Peak RSS of the prover |
|---|---|---|---|
| 1 | Docker pull of the gnark image denied by a stale registry login (first run also downloaded the 7.9 GB Groth16 artifacts) | 16:52 | 14.0 GiB |
| 2 | Image pulled anonymously; gnark's Groth16 step killed by the kernel's OOM killer at 10.4 GB, while the prover still held its memory | 5:48 | 14.0 GiB |
| 3 | 2 workers per stage instead of 8 (`SP1_WORKER_NUM_*`), `MALLOC_ARENA_MAX=2`: still 13.3 GB within a minute; stopped by the session for memory pressure | ~1 min | 13.0 GiB |

Logs: `results/release_groth16_run1_docker_denied.log`, `results/release_groth16_run2_oom.log`,
`results/release_groth16.log`, `results/release_groth16_mem.log` (used MB, prover MB every 5 s).

Reading: the STARK, recursion and wrap stages of a 97.5k-instruction program finished in under
6 minutes (run 2 reached gnark), but the whole Groth16 path needs more than 15 GB as the SDK runs
it. Still open:
- SP1 proving time per stage;
- whether a two-process run (wrap proof saved, then gnark alone) fits 16 GB, which matters
  because after an outbound sunset the user's device makes this proof.

## Real proving in three processes (2026-11-16)

`split` (`script/src/bin/split.rs`) runs each stage in its own process, so no stage holds
another's memory; the Groth16 stage follows sp1-prover 6.10.0's `run_groth16`. Same desktop,
WSL with 15.2 GB of RAM and 4 GB of swap, default worker counts. Memory sampled every 2 s
(`results/split/mem_*.log`); logs in `results/split/`.

| Stage | Output | Wall clock | Peak RSS | Machine |
|---|---|---|---|---|
| `split compress`: core + compressed STARK | `compressed.bin` (1.27 MB), verified | 192 s | 14.1 GiB | 15.4 GB used, 1.9 GB swap |
| `split wrap`: shrink + wrap to BN254 | `wrap.bin` (1.34 MB) | 282 s | 14.2 GiB | 15.4 GB used, 2.3 GB swap |
| `split groth16`: gnark only (Docker, circuit v6.1.0, 15,972,262 constraints) | **killed by the OOM killer** | 128 s | gnark 14.9 GB RAM | swap full (4 GB) |

Reading:
- SP1's Groth16 step alone needs more than 15 GB of RAM plus 4 GB of swap, so it can't run on
  the 16 GB minimum spec, however the stages are arranged. Its real need wasn't measured.
- The two STARK stages fit only just, and the swapping inflated their times. For comparison,
  RISC Zero's whole release Groth16 proof takes 74 s and 2.3 GiB on the same machine.
- Consequences (proposal §14, P.6):
  - the second relay needs a larger machine or a proving service, with no privacy cost;
  - a release after an outbound sunset needs the same, at the cost of that release's privacy
    if a service proves it.

Run: `cd script && cargo build --release && ../target/release/release-script [prove-release | prove-relay range_1x64k]`
(`~/.sp1/bin` on PATH).
