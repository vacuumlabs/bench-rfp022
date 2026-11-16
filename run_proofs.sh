#!/usr/bin/env bash
# Queued real-proving run for the RFP-022 bench (RISC0_DEV_MODE=0, CPU prover).
# Uses all cores and up to ~9-10 GiB RAM per proof; start it when the PC is free.
#   1. LEZ side (RISC Zero 3.0.5): the user's inbound claim proof
#   2. Ethereum side (RISC Zero 3.1.0): the user's release proof and relay blocks, as STARKs
#   3. Groth16 of the release proof (needs Docker reachable from WSL), then the vault test
#      with that real proof against the real RiscZeroGroth16Verifier 3.1.0 on a mainnet fork
# Output: results/proving.csv and one log per step in results/logs/. Safe to re-run.
set -uo pipefail
cd "$(dirname "$0")"
export RISC0_DEV_MODE=0
R031=$HOME/.risc0/extensions/v3.1.0-cargo-risczero-x86_64-unknown-linux-gnu/r0vm
RPC=${MAINNET_RPC:-https://ethereum-rpc.publicnode.com}
mkdir -p results/logs
out=results/proving.csv
echo "step,program,result_line,peak_rss_mib,wall_s" > "$out"

run() { # name, dir, server_path_or_empty, command...
  local name=$1 dir=$2 server=$3; shift 3
  local log=results/logs/$name.log t0 t1 rss line
  echo ">> $name"
  t0=$(date +%s)
  ( cd "$dir" && if [ -n "$server" ]; then RISC0_SERVER_PATH=$server /usr/bin/time -v "$@"; else /usr/bin/time -v "$@"; fi ) > "$log" 2>&1
  local rc=$?
  t1=$(date +%s)
  rss=$(awk '/Maximum resident set size/ {print int($6/1024)}' "$log")
  line=$(grep -E '^(PROVED|EXEC|wrote)' "$log" | tr '\n' ' ' | tr ',' ';')
  [ $rc -ne 0 ] && line="FAILED rc=$rc (see $log)"
  echo "$name,$dir,$line,$rss,$((t1 - t0))" >> "$out"
  echo "   $line  peak ${rss} MiB, $((t1 - t0)) s"
}

# Builds (fast if already built).
(cd lez && cargo build --release -q -p claim-bench) || exit 1
(cd zk && RISC0_SERVER_PATH=$R031 cargo build --release -q -p bridge-bench) || exit 1
for d in 0 31 299; do  # claim with a parent-hash chain of 1, 32 and 300 real mainnet headers
  [ -f lez/results/inputs/claim_d$d.bin ] || (cd lez && ./target/release/claim-bench inputs fixtures/receipts.json fixtures/headers.json $d)
done
[ -f zk/results/inputs/release.bin ] || (cd zk && RISC0_SERVER_PATH=$R031 ./target/release/bridge-bench inputs)

run claim_prove        lez "" ./target/release/claim-bench prove results/inputs/claim_d0.bin
run claim32_prove      lez "" ./target/release/claim-bench prove results/inputs/claim_d31.bin
run claim300_prove     lez "" ./target/release/claim-bench prove results/inputs/claim_d299.bin
run release_prove      zk "$R031" ./target/release/bridge-bench prove release results/inputs/release.bin
run relay64k_prove     zk "$R031" ./target/release/bridge-bench prove relay results/inputs/relay_64k.bin
run relay1m_prove      zk "$R031" ./target/release/bridge-bench prove relay results/inputs/relay_1m.bin

if command -v docker > /dev/null && docker info > /dev/null 2>&1; then
  run release_groth16  zk "$R031" ./target/release/bridge-bench groth16 release results/inputs/release.bin
  run vault_real_proof vault "" forge test --match-contract RealProofTest --fork-url "$RPC" -vv
  grep -E 'execution gas|approx tx gas|PASS|FAIL' results/logs/vault_real_proof.log
else
  echo "Docker not reachable from WSL: skipped the Groth16 step and the real-proof vault test."
  echo "Start Docker Desktop, enable WSL integration for this distro, then re-run this script."
  echo "release_groth16,zk,SKIPPED no docker,," >> "$out"
fi
echo "done: $out"
