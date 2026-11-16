# Isthmus bench (RFP-022, with RFP-021)

These are the prototypes and measurements behind the proposal's §13 tables. This is bench code
written to measure the design before filing. It is not the deliverable and has not been audited.
Results and their history: [`RESULTS.md`](RESULTS.md).

| Folder | What it measures | Proof system |
|---|---|---|
| [`lez/`](lez) | The inbound claim proof the user makes on their device: accumulator membership, deposit-log inclusion in a real mainnet receipt, the parent-hash chain, the opening and the nullifier | RISC Zero 3.0.5 (LEZ v0.4.0-rc3's version and lock) |
| [`zk/`](zk) | The release proof (outbound, the user's) and the relay guests: `relay_block` (one block, used for the proving numbers), `relay_range` (the batch the vault takes), `relay_failsafe` (`relay_range` plus the failsafe's claim nullifiers and counts) | RISC Zero 3.1.0 |
| [`vault/`](vault) | The Solidity vault prototype: deposits, relay, release, caps, freeze, fees; gas on a mainnet fork, including a real Groth16 proof against the real verifier | `risc0-ethereum` v3.1.0 |
| [`bedrock/`](bedrock) | What an Ethereum-side check of LEZ finality would cost if it verified Bedrock consensus: one Proof of Leadership verified in RISC Zero | RISC Zero 3.1.0 |
| [`sp1/`](sp1) | The second release path's cost: an SP1 port of the release and relay, journals compared with RISC Zero's, and SP1's on-chain verifier gas | SP1 6.10, `sp1-contracts` v6.1.0 |
| [`run_proofs.sh`](run_proofs.sh) | Queued real proving (claim, release, relay), Groth16, then the vault test with the real proof | |

The RFP-021 light client's bench (update steps, equivalence test against Helios) is in its
own repository.

## Running

Requirements: Rust with the RISC Zero toolchain (`rzup`, versions 3.0.5 and 3.1.0), Foundry,
Docker for Groth16. For `sp1/`, the SP1 toolchain (`sp1up`).

- Executor only (fast, no proving): `cd lez && cargo run --release -p claim-bench -- execute <input>`,
  `cd zk && cargo run --release -p bridge-bench -- execute <release|range|failsafe> <input>`.
  `inputs` regenerates the input files. `zk/` and `bedrock/` need
  `RISC0_SERVER_PATH` set to the 3.1.0 `r0vm` (see `run_proofs.sh`).
- Real proving: `./run_proofs.sh`. It uses every core and about 9–10 GiB of memory per proof,
  so start it on an otherwise idle machine. Output goes to `results/proving.csv` and `results/logs/`.
- Vault tests: see [`vault/README.md`](vault/README.md).

## Where the bench lags the design

The proposal describes the design after two adversarial reviews. Some of the changes those
reviews produced are in the text but not yet in this bench. Each is built in the milestone
named; none of them changes a measured cost by more than noted.

| Design (proposal section) | Bench today | Milestone |
|---|---|---|
| `Deposit` carries the fee paid and a destination memo; `deposit(token, denom, C, memo)` (§4, §5) | `Deposit(token, denom, C, index)`; the claim takes the fee from the configuration. The memo adds a few hundred gas per deposit | M3 |
| The claim's second nullifier `N2` (§5) | One nullifier `N` | M3 |
| The claim's public part on LEZ: root check, nullifier accounts, cap, mint (§5) | Not built; only the private proof is measured | M1 (measured), M3 |
| `B` commits to `H("rt", s_rt)`; a re-targeting release proves `s_rt` (§4, §6) | `B` without the re-target commitment; no re-targeting release | M4 |
| The release pays a protocol fee, and the relayer fee goes into the relayer pool (§6) | The release pays `denom − relayer_fee` to `dest` and the fee to the named relayer | M4 |
| Real LEZ blocks: the block encoding, receipts and the reverted list (§6) | Synthetic blocks, sized like real ones, with a simplified burn record | M4 |
| Per-token release delay: each root stores its acceptance time (§8) | Not built | M4 |
| Failsafe: sunset, refunds with absence proofs in an indexed claim-nullifier tree, settlement over amounts (§9) | `relay_failsafe` appends claim nullifiers to a plain tree and commits counts, not amounts; no sunset, refunds or settlement in the vault | M4 |
| Second release path: an independently written SP1 relay and release, the vault's second root and tip, the switch at sunset (§9) | A port of our RISC Zero code for measurement only (see `sp1/`); not wired into the vault | M4 |
| Admin and freeze through the RFP-001/RFP-002 authorities, with a two-step admin transfer and immutable bounds (§8) | Single-address admin and freezer; a cap of 0 means "no cap" | M3 |

## License

MIT OR Apache-2.0, at your option: [`LICENSE-MIT`](LICENSE-MIT), [`LICENSE-APACHE`](LICENSE-APACHE). The SP1 scaffolding in `sp1/` comes from Succinct's project template (`sp1/LICENSE-MIT`).
