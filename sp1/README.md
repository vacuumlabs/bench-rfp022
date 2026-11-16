# SP1 second release path: cost measurements

These are measurements for the proposal's second release path (§9). It is a **port** of
`../zk`'s release and relay to SP1 6.10, made only to size the work. The second path we deliver
is written independently of the RISC Zero code; reusing this port would defeat its purpose.
Results: [`RESULTS.md`](RESULTS.md).

| Folder | What |
|---|---|
| `lib/` | The release and relay statements, shared by the SP1 programs and the script |
| `program/`, `relay/` | The SP1 programs (release, relay) |
| `script/` | `release-script`: executes both on `../zk`'s inputs and compares the journals byte for byte with RISC Zero's. `split`: a real Groth16 proof of the release in three separate processes (compress, wrap, groth16) |
| `contracts/` | Foundry tests: `SP1VerifierGroth16` v6 gas on SP1's official test proof, and on our own release proof once `fixtures/release-groth16.json` exists |
| `results/` | Logs and memory samples of the proving runs |

Run, with `~/.sp1/bin` on PATH:

```
cd script && cargo build --release
../target/release/release-script                   # execute both, compare journals
../target/release/split compress|wrap|groth16      # real proof, one stage per process
cd ../contracts && forge test -vv                  # verifier gas
```

SP1's Groth16 step runs in the `ghcr.io/succinctlabs/sp1-gnark:v6.1.0` Docker image and needs
more memory than our 15 GB test machine has (see `RESULTS.md`). Without it, `split groth16` and
`SP1ReleaseGas` cannot complete.
