// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.26;

import {Test, console2} from "forge-std/Test.sol";
import {IERC20} from "openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IRiscZeroVerifier, Receipt as RiscZeroReceipt} from "risc0/IRiscZeroVerifier.sol";
import {ControlID, RiscZeroGroth16Verifier} from "risc0/groth16/RiscZeroGroth16Verifier.sol";
import {TestReceipt} from "risc0-test/TestReceiptV3_1.sol";
import {BridgeVault, ILezFinality, IWETH} from "../src/BridgeVault.sol";

/// Accepts every proof: isolates the vault's own gas from proof verification.
contract AcceptAllVerifier is IRiscZeroVerifier {
    function verify(bytes calldata, bytes32, bytes32) external pure {}
    function verifyIntegrity(RiscZeroReceipt calldata) external pure {}
}

/// Stand-in for Ethereum-visible LEZ finality: blocks are final once marked.
contract MockLezFinality is ILezFinality {
    mapping(bytes32 => bool) public isFinal;

    function setFinal(bytes32 h) external {
        isFinal[h] = true;
    }

    function isFinalLezBlock(bytes32 h) external view returns (bool) {
        return isFinal[h];
    }
}

/// Gas on an Ethereum mainnet fork with the real USDC, USDT and WETH contracts.
/// Run: forge test --fork-url <mainnet RPC> --fork-block-number 26398402 -vv
contract BridgeVaultGasTest is Test {
    address constant USDC = 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48;
    address constant USDT = 0xdAC17F958D2ee523a2206206994597C13D831ec7;
    address constant WETH = 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2;
    bytes32 constant GENESIS = bytes32(0);
    bytes32 constant ROOT = keccak256("burn-root-1");

    bytes32 constant TIP0 = keccak256("lez-genesis");
    BridgeVault vault;
    bytes32 CFG; // cached: an external cfg() call would consume a pending expectRevert
    MockLezFinality fin;
    address user = makeAddr("user");
    address dest = makeAddr("dest");
    address relayer = makeAddr("relayer");

    function setUp() public {
        fin = new MockLezFinality();
        BridgeVault.Pairing memory p = BridgeVault.Pairing(keccak256("lez"), keccak256("zone"), keccak256("program"), keccak256("rfp021"));
        vault = new BridgeVault(new AcceptAllVerifier(), bytes32(uint256(1)), bytes32(uint256(2)), p, IWETH(WETH), fin, GENESIS, TIP0, 100);
        CFG = vault.cfg();
        uint256[] memory stable = new uint256[](3);
        (stable[0], stable[1], stable[2]) = (100e6, 1_000e6, 10_000e6);
        uint256[] memory eth = new uint256[](3);
        (eth[0], eth[1], eth[2]) = (0.1 ether, 1 ether, 10 ether);
        vault.register(USDC, stable, 1_000_000e6, 1_000_000e6);
        vault.register(USDT, stable, 1_000_000e6, 1_000_000e6);
        vault.register(WETH, eth, 1_000 ether, 1_000 ether);
        vault.setRelayerFee(USDC, 1_000e6, 1e6);
        vault.setRelayerFee(WETH, 1 ether, 0.001 ether);
        // Inbound fee on top of the denomination (claim mints the full denomination on LEZ).
        vault.setClaimFee(USDC, 1_000e6, 1e6);
        vault.setClaimFee(USDT, 1_000e6, 1e6);
        vault.setClaimFee(WETH, 1 ether, 0.001 ether);
        deal(USDC, user, 100_000e6);
        deal(USDT, user, 100_000e6, true);
        vm.deal(user, 100 ether);
        vm.startPrank(user);
        IERC20(USDC).approve(address(vault), type(uint256).max);
        // USDT's approve returns nothing: call it low-level.
        (bool ok,) = USDT.call(abi.encodeWithSignature("approve(address,uint256)", address(vault), type(uint256).max));
        require(ok);
        vm.stopPrank();
    }

    function _calldataGas(bytes memory data) internal pure returns (uint256 g) {
        for (uint256 i; i < data.length; ++i) {
            g += data[i] == 0 ? 4 : 16;
        }
    }

    function _report(string memory name, uint256 exec, bytes memory data) internal pure {
        // Transaction total = 21,000 base + calldata + execution (no access list).
        console2.log(name);
        console2.log("  execution gas", exec);
        console2.log("  approx tx gas", 21_000 + _calldataGas(data) + exec);
    }

    function _deposit(address token, uint256 denom, bytes32 c) internal returns (uint256 used) {
        vm.prank(user);
        uint256 g = gasleft();
        vault.deposit(token, denom, c);
        used = g - gasleft();
    }

    function test_gas_deposits() public {
        // Second deposit of each token = the vault's token balance slot already warm in storage.
        _deposit(USDC, 1_000e6, keccak256("c0"));
        uint256 g = _deposit(USDC, 1_000e6, keccak256("c1"));
        _report("deposit USDC 1,000 (vault already holds USDC)", g, abi.encodeCall(vault.deposit, (USDC, 1_000e6, keccak256("c1"))));
        _deposit(USDT, 1_000e6, keccak256("c2"));
        g = _deposit(USDT, 1_000e6, keccak256("c3"));
        _report("deposit USDT 1,000", g, abi.encodeCall(vault.deposit, (USDT, 1_000e6, keccak256("c3"))));
        vm.prank(user);
        vault.depositETH{value: 1.001 ether}(1 ether, keccak256("e0"));
        vm.prank(user);
        uint256 g0 = gasleft();
        vault.depositETH{value: 1.001 ether}(1 ether, keccak256("e1"));
        g = g0 - gasleft();
        _report("depositETH 1 (wrapped to WETH)", g, abi.encodeCall(vault.depositETH, (1 ether, keccak256("e1"))));
    }

    function _journal(bytes32 nullifier, address token, uint256 denom, uint8 asNative, uint256 fee) internal view returns (bytes memory) {
        return abi.encode(BridgeVault.ReleaseJournal(CFG, ROOT, nullifier, dest, asNative, token, denom, relayer, fee));
    }

    function _relayJournal(bytes32 prevRoot, bytes32 root, bytes32 prevTip, bytes32 tip, uint64 height) internal view returns (bytes memory) {
        return abi.encode(CFG, prevRoot, root, prevTip, tip, height, uint64(4));
    }

    function _acceptRoot() internal {
        bytes32 tip = keccak256("lez-block-116");
        fin.setFinal(tip);
        vault.relay(hex"00", _relayJournal(GENESIS, ROOT, TIP0, tip, 116));
    }

    function test_gas_relay_and_release() public {
        // Escrow first, so releases pay out of real balances.
        _deposit(USDC, 10_000e6, keccak256("c0"));
        vm.prank(user);
        vault.depositETH{value: 10 ether}(10 ether, keccak256("e0"));

        // Two consecutive batches: the second is the steady state (tip slots already non-zero).
        bytes32 tip1 = keccak256("lez-block-116");
        bytes32 tip2 = keccak256("lez-block-132");
        fin.setFinal(tip1);
        fin.setFinal(tip2);
        bytes32 root0 = keccak256("burn-root-0");
        vault.relay(hex"00", _relayJournal(GENESIS, root0, TIP0, tip1, 116));
        bytes memory rj = _relayJournal(root0, ROOT, tip1, tip2, 132);
        uint256 g = gasleft();
        vault.relay(hex"00", rj);
        _report("relay, any number of LEZ blocks (verifier excluded)", g - gasleft(), abi.encodeCall(vault.relay, (hex"00", rj)));

        bytes memory j = _journal(keccak256("n1"), USDC, 1_000e6, 0, 1e6);
        g = gasleft();
        vault.release(hex"00", j);
        _report("release USDC 1,000 with relayer fee (verifier excluded)", g - gasleft(), abi.encodeCall(vault.release, (hex"00", j)));
        assertEq(IERC20(USDC).balanceOf(dest), 999e6);
        assertEq(IERC20(USDC).balanceOf(relayer), 1e6);

        j = _journal(keccak256("n2"), WETH, 1 ether, 1, 0.001 ether);
        g = gasleft();
        vault.release(hex"00", j);
        _report("release 1 ETH as native ETH with relayer fee (verifier excluded)", g - gasleft(), abi.encodeCall(vault.release, (hex"00", j)));
        assertEq(dest.balance, 0.999 ether);
    }

    function test_rejects() public {
        _deposit(USDC, 1_000e6, keccak256("c0"));
        _acceptRoot();
        bytes memory j = _journal(keccak256("n1"), USDC, 1_000e6, 0, 1e6);
        vault.release(hex"00", j);
        vm.expectRevert(BridgeVault.Rejected.selector); // replayed nullifier (R.3)
        vault.release(hex"00", j);
        bytes memory wrongCfg = abi.encode(BridgeVault.ReleaseJournal(keccak256("other"), ROOT, keccak256("n2"), dest, 0, USDC, 1_000e6, relayer, 1e6));
        vm.expectRevert(BridgeVault.Rejected.selector); // other configuration (F.15)
        vault.release(hex"00", wrongCfg);
        bytes memory unknownRoot = abi.encode(BridgeVault.ReleaseJournal(CFG, keccak256("x"), keccak256("n3"), dest, 0, USDC, 1_000e6, relayer, 1e6));
        vm.expectRevert(BridgeVault.Rejected.selector); // root never accepted
        vault.release(hex"00", unknownRoot);
        vm.expectRevert(BridgeVault.Rejected.selector); // reused commitment
        vm.prank(user);
        vault.deposit(USDC, 1_000e6, keccak256("c0"));
        vm.expectRevert(BridgeVault.Unsupported.selector); // not a permitted denomination (F.8)
        vm.prank(user);
        vault.deposit(USDC, 1_234e6, keccak256("c9"));
        vm.expectRevert(BridgeVault.Unsupported.selector); // relayer fee other than the fixed one (DR-11)
        vault.release(hex"00", _journal(keccak256("n5"), USDC, 1_000e6, 0, 2e6));
        bytes32 tip = keccak256("lez-block-116");
        bytes32 next = keccak256("lez-block-120");
        vm.expectRevert(BridgeVault.Rejected.selector); // batch not extending the relayed tip
        vault.relay(hex"00", _relayJournal(ROOT, keccak256("r2"), keccak256("elsewhere"), next, 120));
        vm.expectRevert(BridgeVault.Rejected.selector); // tip not final
        vault.relay(hex"00", _relayJournal(ROOT, keccak256("r2"), tip, next, 120));
        fin.setFinal(next);
        vm.expectRevert(BridgeVault.Rejected.selector); // height not advancing
        vault.relay(hex"00", _relayJournal(ROOT, keccak256("r2"), tip, next, 116));
        vault.relay(hex"00", _relayJournal(ROOT, keccak256("r2"), tip, next, 120));
        assertTrue(vault.burnRoot(ROOT), "older roots stay accepted (R.4)");
        vault.setFreeze(false, true);
        vm.expectRevert(BridgeVault.Frozen.selector); // release frozen independently (F.14)
        vault.release(hex"00", _journal(keccak256("n4"), USDC, 1_000e6, 0, 1e6));
    }

    function test_review1_fixes() public {
        // Inbound fee is charged on top: escrow holds denom + fee, the claim mints the full denom.
        uint256 before = IERC20(USDC).balanceOf(address(vault));
        _deposit(USDC, 1_000e6, keccak256("c0"));
        assertEq(IERC20(USDC).balanceOf(address(vault)) - before, 1_001e6);
        vm.expectRevert(BridgeVault.Unsupported.selector); // ETH without the fee on top
        vm.prank(user);
        vault.depositETH{value: 1 ether}(1 ether, keccak256("e9"));
        // The same C under another token is accepted: nobody can block a deposit by front-running its C.
        _deposit(USDT, 1_000e6, keccak256("c0"));
        assertEq(vault.cfg(), sha256(abi.encodePacked("cfg", uint64(block.chainid), address(vault), keccak256("lez"), keccak256("zone"), keccak256("program"), keccak256("rfp021"))));

        _deposit(USDC, 1_000e6, keccak256("c1"));
        _deposit(USDC, 1_000e6, keccak256("c2"));
        _acceptRoot();
        // A fee change keeps the old fee valid for FEE_GRACE, so proofs already made still release.
        vault.setRelayerFee(USDC, 1_000e6, 2e6);
        vault.release(hex"00", _journal(keccak256("old-fee"), USDC, 1_000e6, 0, 1e6));
        vault.release(hex"00", _journal(keccak256("new-fee"), USDC, 1_000e6, 0, 2e6));
        vm.warp(block.timestamp + 7 days);
        vm.expectRevert(BridgeVault.Unsupported.selector); // old fee expired
        vault.release(hex"00", _journal(keccak256("old-fee-late"), USDC, 1_000e6, 0, 1e6));

        // After a chain split the vault on the other chain refuses every proof.
        vm.chainId(10_001);
        vm.expectRevert(BridgeVault.Rejected.selector);
        vault.release(hex"00", _journal(keccak256("split"), USDC, 1_000e6, 0, 2e6));
    }
}

/// No fork needed.
contract RealVerifierGasTest is Test {
    /// The real RISC Zero 3.1 Groth16 verifier on its official 3.1 test receipt.
    function test_gas_real_groth16_verifier_v3_1() public {
        RiscZeroGroth16Verifier v = new RiscZeroGroth16Verifier(ControlID.CONTROL_ROOT, ControlID.BN254_CONTROL_ID);
        uint256 g = gasleft();
        v.verify(TestReceipt.SEAL, TestReceipt.IMAGE_ID, sha256(TestReceipt.JOURNAL));
        uint256 used = g - gasleft();
        console2.log("RiscZeroGroth16Verifier 3.1.0 verify, execution gas", used);
        // A tampered journal must fail.
        bytes32 tampered = sha256(abi.encodePacked(TestReceipt.JOURNAL, uint8(1)));
        vm.expectRevert();
        v.verify(TestReceipt.SEAL, TestReceipt.IMAGE_ID, tampered);
    }
}
