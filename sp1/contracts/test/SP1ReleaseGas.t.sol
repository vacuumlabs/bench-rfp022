// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import {Test, console2} from "forge-std/Test.sol";
import {SP1Verifier} from "sp1/v6.1.0/SP1VerifierGroth16.sol";

/// Gas of verifying the SP1 release proof (second release path, §9) with the pinned v6.1.0
/// Groth16 verifier, called directly (no gateway), as the vault would after an outbound sunset.
contract SP1ReleaseGas is Test {
    SP1Verifier verifier;
    bytes32 vkey;
    bytes publicValues;
    bytes proof;

    function setUp() public {
        string memory j = vm.readFile("fixtures/release-groth16.json");
        vkey = vm.parseJsonBytes32(j, ".vkey");
        publicValues = vm.parseJsonBytes(j, ".publicValues");
        proof = vm.parseJsonBytes(j, ".proof");
        verifier = new SP1Verifier();
    }

    function test_verify_gas() public view {
        uint256 g = gasleft();
        verifier.verifyProof(vkey, publicValues, proof);
        console2.log("SP1 Groth16 verifyProof gas (in-call, warm):", g - gasleft());
    }

    function test_reject_tampered() public {
        bytes memory pv = publicValues;
        pv[40] ^= 0x01;
        vm.expectRevert();
        verifier.verifyProof(vkey, pv, proof);
    }
}
