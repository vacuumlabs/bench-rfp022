#!/usr/bin/env bash
# Real-transaction gas on an anvil fork of Ethereum mainnet (real USDC, USDT, WETH).
# Each operation is its own transaction; gasUsed comes from its receipt.
# Usage: scripts/gas_anvil.sh [mainnet RPC]   Output: results/gas.csv
set -euo pipefail
cd "$(dirname "$0")/.."
RPC_UP=${1:-https://ethereum-rpc.publicnode.com}
PORT=8546
RPC=http://127.0.0.1:$PORT
BLOCK=$(cast block-number --rpc-url "$RPC_UP")
anvil --fork-url "$RPC_UP" --fork-block-number "$BLOCK" --port $PORT --silent &
ANVIL=$!
trap 'kill $ANVIL' EXIT
sleep 4

USDC=0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48
USDT=0xdAC17F958D2ee523a2206206994597C13D831ec7
WETH=0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2
# anvil's first default account (public test key)
PK=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
ME=$(cast wallet address --private-key $PK)
DEST=0x000000000000000000000000000000000000dEaD
RELAYER=0x0000000000000000000000000000000000000B0b

field() { python3 -c "import json,sys; v=json.load(sys.stdin)['$1']; print(int(v,16) if '$1'=='gasUsed' and str(v).startswith('0x') else v)"; }
send() { cast send --rpc-url $RPC --private-key $PK "$@" --json | field gasUsed; }
deploy() { forge create --rpc-url $RPC --private-key $PK --broadcast --json "$@" | field deployedTo; }

# Fund ME with USDC (FiatToken balances at slot 9) and USDT (balances at slot 2).
for pair in "$USDC 9" "$USDT 2"; do
  set -- $pair
  slot=$(cast index address "$ME" "$2")
  cast rpc --rpc-url $RPC anvil_setStorageAt "$1" "$slot" "$(cast to-uint256 1000000000000)" > /dev/null
done

VERIFIER=$(deploy test/BridgeVault.t.sol:AcceptAllVerifier)
FIN=$(deploy test/BridgeVault.t.sol:MockLezFinality)
TIP0=$(cast keccak lez-genesis)
PAIRING="($(cast keccak lez),$(cast keccak zone),$(cast keccak program),$(cast keccak rfp021))"
VAULT=$(deploy src/BridgeVault.sol:BridgeVault --constructor-args "$VERIFIER" 0x0000000000000000000000000000000000000000000000000000000000000001 0x0000000000000000000000000000000000000000000000000000000000000002 "$PAIRING" $WETH "$FIN" 0x0000000000000000000000000000000000000000000000000000000000000000 "$TIP0" 100)
CFG=$(cast call --rpc-url $RPC $VAULT "cfg()(bytes32)")
send $VAULT "register(address,uint256[],uint256,uint256)" $USDC "[100000000,1000000000,10000000000]" 1000000000000 1000000000000 > /dev/null
send $VAULT "register(address,uint256[],uint256,uint256)" $USDT "[100000000,1000000000,10000000000]" 1000000000000 1000000000000 > /dev/null
send $VAULT "register(address,uint256[],uint256,uint256)" $WETH "[100000000000000000,1000000000000000000,10000000000000000000]" 1000000000000000000000 1000000000000000000000 > /dev/null
send $VAULT "setRelayerFee(address,uint256,uint256)" $USDC 1000000000 1000000 > /dev/null
send $VAULT "setRelayerFee(address,uint256,uint256)" $USDT 1000000000 1000000 > /dev/null
send $VAULT "setRelayerFee(address,uint256,uint256)" $WETH 1000000000000000000 1000000000000000 > /dev/null
# Inbound fee on top of the denomination (the claim mints the full denomination on LEZ).
send $VAULT "setClaimFee(address,uint256,uint256)" $USDC 1000000000 1000000 > /dev/null
send $VAULT "setClaimFee(address,uint256,uint256)" $USDT 1000000000 1000000 > /dev/null
send $VAULT "setClaimFee(address,uint256,uint256)" $WETH 1000000000000000000 1000000000000000 > /dev/null
send $USDC "approve(address,uint256)" $VAULT 1000000000000 > /dev/null
send $USDT "approve(address,uint256)" $VAULT 1000000000000 > /dev/null

out=results/gas.csv
mkdir -p results
echo "operation,gas_used,note" > $out
row() { echo "$1,$2,$3" >> $out; echo "$1: $2 ($3)"; }

row deposit_usdc_first "$(send $VAULT "deposit(address,uint256,bytes32)" $USDC 1000000000 $(cast keccak c0))" "first USDC deposit into the vault"
row deposit_usdc "$(send $VAULT "deposit(address,uint256,bytes32)" $USDC 1000000000 $(cast keccak c1))" "typical: vault already holds USDC"
send $VAULT "deposit(address,uint256,bytes32)" $USDT 1000000000 $(cast keccak c2) > /dev/null
row deposit_usdt "$(send $VAULT "deposit(address,uint256,bytes32)" $USDT 1000000000 $(cast keccak c3))" "typical"
send $VAULT "depositETH(uint256,bytes32)" 1000000000000000000 $(cast keccak e0) --value 1.001ether > /dev/null
row deposit_eth "$(send $VAULT "depositETH(uint256,bytes32)" 1000000000000000000 $(cast keccak e1) --value 1.001ether)" "typical, wrapped to WETH"
send $VAULT "depositETH(uint256,bytes32)" 10000000000000000000 $(cast keccak e2) --value 10ether > /dev/null

# Relay batches: each covers the contiguous LEZ range after the vault's tip; gas does not
# depend on the number of blocks in the range. First batch, then the steady state.
height=100
for n in 1 2 3; do
  prev=$(cast call --rpc-url $RPC $VAULT "latestBurnRoot()(bytes32)")
  ptip=$(cast call --rpc-url $RPC $VAULT "lezTip()(bytes32)")
  root=$(cast keccak "burn-root-$n")
  tip=$(cast keccak "lez-tip-$n")
  height=$((height + 16))
  send $FIN "setFinal(bytes32)" "$tip" > /dev/null
  journal=$(cast abi-encode "f(bytes32,bytes32,bytes32,bytes32,bytes32,uint64,uint64)" "$CFG" "$prev" "$root" "$ptip" "$tip" $height 4)
  g=$(send $VAULT "relay(bytes,bytes)" 0x00 "$journal")
  [ $n -eq 1 ] && row relay_batch_first "$g" "first batch; verifier excluded (accept-all mock); finality mock"
  [ $n -eq 3 ] && row relay_batch "$g" "steady state; any number of blocks; verifier excluded; finality mock"
done
ROOT=$(cast keccak burn-root-3)

rel() { cast abi-encode "f(bytes32,bytes32,bytes32,address,uint8,address,uint256,address,uint256)" "$CFG" "$ROOT" "$1" $DEST "$2" "$3" "$4" $RELAYER "$5"; }
row release_usdc_first "$(send $VAULT "release(bytes,bytes)" 0x00 "$(rel $(cast keccak n0) 0 $USDC 1000000000 1000000)")" "first release; dest and relayer new USDC holders; verifier excluded"
row release_usdc "$(send $VAULT "release(bytes,bytes)" 0x00 "$(rel $(cast keccak n1) 0 $USDC 1000000000 1000000)")" "dest and relayer already hold USDC; verifier excluded"
row release_usdt "$(send $VAULT "release(bytes,bytes)" 0x00 "$(rel $(cast keccak n2) 0 $USDT 1000000000 1000000)")" "first USDT to dest and relayer; verifier excluded"
row release_eth_native "$(send $VAULT "release(bytes,bytes)" 0x00 "$(rel $(cast keccak n3) 1 $WETH 1000000000000000000 1000000000000000)")" "paid as native ETH; verifier excluded"
echo "fork_block,$BLOCK," >> $out
echo "wrote $out (fork block $BLOCK)"
