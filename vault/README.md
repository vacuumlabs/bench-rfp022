# Vault prototype

`src/BridgeVault.sol` is a bench prototype of the Ethereum vault. It covers:
- deposits of ERC-20 tokens and ETH (held as WETH);
- the relay, taking a contiguous range of LEZ blocks after the vault's tip;
- releases through the pinned `RiscZeroGroth16Verifier` 3.1.0;
- caps, freeze, the relayer fee with a 7-day grace period, and the chain-split refusal.

It is not audited code. Where it lags the proposal's design, see the table in
[`../README.md`](../README.md).

| Test | What |
|---|---|
| `BridgeVaultGasTest.test_gas_deposits` | Deposit gas for USDC, USDT and ETH on a mainnet fork |
| `BridgeVaultGasTest.test_gas_relay_and_release` | Relay batch and release gas, with an accept-all verifier (verifier excluded) |
| `BridgeVaultGasTest.test_rejects` | Replay, another configuration, unknown root, reused commitment, unpermitted denomination or relayer fee, a relay batch that doesn't extend the tip, isn't final or doesn't advance, frozen releases |
| `BridgeVaultGasTest.test_review1_fixes` | Fixes from the first review: fee on top at deposit, the same `C` under another token, relayer-fee grace, chain split |
| `RealVerifierGasTest` | The real verifier on RISC Zero's 3.1 test receipt (no fork needed) |
| `RealProofTest` | A real Groth16 proof of our release guest (`proofs/release.json`) against the real verifier, on a fork |

Run (Foundry):

```
forge test --fork-url <mainnet RPC> -vv
```

The §13 figures were measured at block 26398402 (`--fork-block-number 26398402`). Pinning that
block now needs an archive RPC; at the current block the tests pass the same.
`scripts/gas_anvil.sh [RPC]` measures the same operations as real transactions on an anvil
fork and writes `results/gas.csv`.
