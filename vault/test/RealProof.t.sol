// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.26;

import {Test, console2} from "forge-std/Test.sol";
import {IERC20} from "openzeppelin/contracts/token/ERC20/IERC20.sol";
import {ControlID, RiscZeroGroth16Verifier} from "risc0/groth16/RiscZeroGroth16Verifier.sol";
import {BridgeVault, ILezFinality, IWETH} from "../src/BridgeVault.sol";

/// Release with a real Groth16 proof from ../zk (bridge-bench groth16 release) against the real
/// RiscZeroGroth16Verifier 3.1.0, on an Ethereum mainnet fork with the real USDC contract.
/// Run: forge test --match-contract RealProofTest --fork-url <mainnet RPC> -vv
contract RealProofTest is Test {
    address constant USDC = 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48;
    address constant WETH = 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2;

    function test_release_with_real_groth16_proof() public {
        string memory p = vm.readFile("proofs/release.json");
        bytes memory seal = vm.parseJsonBytes(p, ".seal");
        bytes memory journal = vm.parseJsonBytes(p, ".journal");
        bytes32 imageId = vm.parseJsonBytes32(p, ".image_id");
        BridgeVault.ReleaseJournal memory j = abi.decode(journal, (BridgeVault.ReleaseJournal));

        RiscZeroGroth16Verifier verifier = new RiscZeroGroth16Verifier(ControlID.CONTROL_ROOT, ControlID.BN254_CONTROL_ID);
        // The proof's cfg was computed for this vault's address on chain 1 (bench/zk host).
        BridgeVault.Pairing memory pairing = BridgeVault.Pairing(sha256("lez"), sha256("zone"), sha256("program"), sha256("rfp021"));
        BridgeVault vault = new BridgeVault(verifier, imageId, bytes32(0), pairing, IWETH(WETH), ILezFinality(address(0)), bytes32(0), bytes32(0), 0);
        console2.log("vault address", address(vault));
        assertEq(j.cfg, vault.cfg(), "proof made for another vault address or chain");
        uint256[] memory d = new uint256[](1);
        d[0] = j.denom;
        vault.register(j.token, d, 0, 0);
        vault.setRelayerFee(j.token, j.denom, j.relayerFee);

        // The burn root the proof was made against, as if relayed; and escrow to pay from.
        bytes32 slot = keccak256(abi.encode(j.root, uint256(13))); // burnRoot mapping slot (forge inspect BridgeVault storage)
        vm.store(address(vault), slot, bytes32(uint256(1)));
        assertTrue(vault.burnRoot(j.root), "storage slot of burnRoot changed: update it");
        deal(USDC, address(vault), j.denom);
        vault.setClaimFee(j.token, j.denom, 0);

        // Same proof with one byte of dest changed (all other checks pass): the verifier rejects it.
        bytes memory tampered = bytes.concat(journal);
        tampered[120] = tampered[120] ^ 0x01;
        vm.expectRevert();
        vault.release(seal, tampered);

        uint256 before = IERC20(USDC).balanceOf(j.dest); // the fork's dest address may already hold USDC
        uint256 g = gasleft();
        vault.release(seal, journal);
        uint256 used = g - gasleft();
        uint256 cd;
        bytes memory call = abi.encodeCall(vault.release, (seal, journal));
        for (uint256 i; i < call.length; ++i) cd += call[i] == 0 ? 4 : 16;
        console2.log("release with real Groth16 proof, execution gas", used);
        console2.log("approx tx gas (21k + calldata + execution)", 21_000 + cd + used);
        assertEq(IERC20(USDC).balanceOf(j.dest) - before, j.denom - j.relayerFee);

        // Same proof again: nullifier spent.
        vm.expectRevert(BridgeVault.Rejected.selector);
        vault.release(seal, journal);
    }
}
