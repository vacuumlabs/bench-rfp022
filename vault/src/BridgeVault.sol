// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.26;

import {IERC20} from "openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {IRiscZeroVerifier} from "risc0/IRiscZeroVerifier.sol";

/// Ethereum-visible LEZ finality (PLATFORM-ASSUMPTIONS): fixed at deployment.
interface ILezFinality {
    function isFinalLezBlock(bytes32 blockHash) external view returns (bool);
}

interface IWETH is IERC20 {
    function deposit() external payable;
    function withdraw(uint256) external;
}

/// Measurement prototype of the RFP-022 vault (design: the Isthmus proposal; gaps: ../../README.md).
/// Everything that decides what is accepted is immutable: verifier, image ids, pairing.
/// Admin and freeze are minimal single-address roles here; the proposal uses the
/// RFP-001/002 semantics. Not audited, not for deployment.
contract BridgeVault {
    using SafeERC20 for IERC20;

    // ---- immutable pairing and verification (Sec.8, F.15) ----
    IRiscZeroVerifier public immutable verifier; // one pinned RiscZeroGroth16Verifier, no router
    bytes32 public immutable releaseImageId;
    bytes32 public immutable relayImageId;
    bytes32 public immutable cfg; // sha256("cfg", chain id, this vault, LEZ chain id, zone id, program id, RFP-021 instance)
    uint256 public immutable chainId; // the chain cfg was computed on; proofs are refused after a chain split
    IWETH public immutable weth;
    ILezFinality public immutable finality;
    uint256 public constant MAX_FEE_BPS = 100; // immutable ceiling on each fee (1%)
    uint256 public constant FEE_GRACE = 7 days; // a replaced relayer fee stays valid this long

    // ---- roles ----
    address public admin;
    address public freezer;

    // ---- registry: token => denomination => permitted ----
    mapping(address => mapping(uint256 => bool)) public permitted;
    mapping(address => bool) public listed;

    // ---- freeze (F.14): global and per token, deposits and releases independently ----
    bool public depositsFrozen;
    bool public releasesFrozen;
    mapping(address => bool) public tokenFrozen;

    // ---- caps (F.11): rolling 24 h window per token, both directions ----
    // Sliding-window counter: the previous window's total counts in proportion to how much
    // of it still overlaps the last 24 h. One packed slot per token and direction.
    uint256 internal constant WINDOW = 24 hours;
    struct Window {
        uint32 start; // start of the current window
        uint112 prev; // total of the previous window
        uint112 cur; // total of the current window
    }
    mapping(address => uint256) public depositCap;
    mapping(address => uint256) public releaseCap;
    mapping(address => Window) internal depositWindow;
    mapping(address => Window) internal releaseWindow;

    // ---- inbound ----
    // Keyed by (token, denom, C): a reused C would publicly link deposits, and keying per
    // token stops anyone from blocking a deposit by front-running its C under another token.
    mapping(bytes32 => bool) public commitmentUsed;
    // Inbound fee charged on top of the denomination at deposit, so the claim mints the full
    // denomination to the user and the fee to the relayer and fee recipient on LEZ: a user
    // can always burn exactly what they minted (fixed denominations, both directions).
    mapping(address => mapping(uint256 => uint256)) public claimFee;
    uint256 public depositCount;

    // ---- outbound ----
    mapping(bytes32 => bool) public burnRoot; // every accepted LEZ burn root, kept forever (R.4)
    bytes32 public latestBurnRoot;
    bytes32 public lezTip; // last relayed LEZ block; the next batch must extend it
    mapping(bytes32 => bool) public burnNullifierSpent;
    uint64 public lezTipHeight;
    // One fixed release relayer fee per (token, denomination): non-distinguishing (DR-11).
    mapping(address => mapping(uint256 => uint256)) public relayerFee;
    mapping(address => mapping(uint256 => uint256)) public prevRelayerFee;
    mapping(address => mapping(uint256 => uint256)) public prevRelayerFeeUntil;

    /// The LEZ side of the pairing; the EVM side is this vault on this chain.
    struct Pairing {
        bytes32 lezChainId;
        bytes32 zoneId;
        bytes32 programId;
        bytes32 rfp021Instance;
    }

    event Deposit(address indexed token, uint256 indexed denom, bytes32 commitment, uint256 index);
    event BurnRootAccepted(bytes32 root, bytes32 tip, uint64 height, uint64 burns);
    event Released(address indexed token, uint256 indexed denom);

    error Unsupported();
    error Frozen();
    error CapExceeded();
    error Rejected();

    constructor(
        IRiscZeroVerifier verifier_,
        bytes32 releaseImageId_,
        bytes32 relayImageId_,
        Pairing memory pairing,
        IWETH weth_,
        ILezFinality finality_,
        bytes32 genesisBurnRoot,
        bytes32 genesisLezTip,
        uint64 genesisLezHeight
    ) {
        verifier = verifier_;
        releaseImageId = releaseImageId_;
        relayImageId = relayImageId_;
        chainId = block.chainid;
        cfg = sha256(
            abi.encodePacked(
                "cfg", uint64(block.chainid), address(this), pairing.lezChainId, pairing.zoneId, pairing.programId, pairing.rfp021Instance
            )
        );
        weth = weth_;
        finality = finality_;
        admin = msg.sender;
        freezer = msg.sender;
        burnRoot[genesisBurnRoot] = true;
        latestBurnRoot = genesisBurnRoot;
        lezTip = genesisLezTip;
        lezTipHeight = genesisLezHeight;
    }

    receive() external payable {
        require(msg.sender == address(weth));
    }

    // ------------------------------------------------------------------ admin
    modifier onlyAdmin() {
        if (msg.sender != admin) revert Rejected();
        _;
    }

    modifier onlyFreezer() {
        if (msg.sender != freezer) revert Rejected();
        _;
    }

    function register(address token, uint256[] calldata denoms, uint256 depCap, uint256 relCap) external onlyAdmin {
        listed[token] = true;
        for (uint256 i; i < denoms.length; ++i) {
            permitted[token][denoms[i]] = true;
        }
        depositCap[token] = depCap;
        releaseCap[token] = relCap;
    }

    /// Delisting stops new deposits only; claims and releases continue (R.4).
    function delist(address token) external onlyAdmin {
        listed[token] = false;
    }

    /// The previous fee stays valid for FEE_GRACE, so a fee change never invalidates a release
    /// proof already made (the fee is a proved journal field).
    function setRelayerFee(address token, uint256 denom, uint256 fee) external onlyAdmin {
        if (!permitted[token][denom] || fee * 10_000 > denom * MAX_FEE_BPS) revert Unsupported();
        prevRelayerFee[token][denom] = relayerFee[token][denom];
        prevRelayerFeeUntil[token][denom] = block.timestamp + FEE_GRACE;
        relayerFee[token][denom] = fee;
    }

    /// Changes apply to new deposits only; the claim of an existing deposit pays the fee its
    /// depositor already paid on top.
    function setClaimFee(address token, uint256 denom, uint256 fee) external onlyAdmin {
        if (!permitted[token][denom] || fee * 10_000 > denom * MAX_FEE_BPS) revert Unsupported();
        claimFee[token][denom] = fee;
    }

    function setFreeze(bool deposits, bool releases) external onlyFreezer {
        depositsFrozen = deposits;
        releasesFrozen = releases;
    }

    function setTokenFrozen(address token, bool frozen) external onlyFreezer {
        tokenFrozen[token] = frozen;
    }

    // ---------------------------------------------------------------- inbound
    /// F.1, F.2: the deposit carries only a hiding commitment C, never the LEZ destination.
    function deposit(address token, uint256 denom, bytes32 commitment) external {
        _checkDeposit(token, denom, commitment);
        uint256 amount = denom + claimFee[token][denom];
        uint256 before = IERC20(token).balanceOf(address(this));
        IERC20(token).safeTransferFrom(msg.sender, address(this), amount); // non-boolean transfer safe (F.17)
        if (IERC20(token).balanceOf(address(this)) - before != amount) revert Unsupported(); // no fee-on-transfer
        _recordDeposit(token, denom, commitment);
    }

    /// Native ETH, held as WETH so ETH and WETH share one wrapped mint and anonymity set.
    function depositETH(uint256 denom, bytes32 commitment) external payable {
        if (msg.value != denom + claimFee[address(weth)][denom]) revert Unsupported();
        _checkDeposit(address(weth), denom, commitment);
        weth.deposit{value: msg.value}();
        _recordDeposit(address(weth), denom, commitment);
    }

    function _checkDeposit(address token, uint256 denom, bytes32 commitment) internal {
        if (!listed[token] || !permitted[token][denom]) revert Unsupported();
        if (depositsFrozen || tokenFrozen[token]) revert Frozen();
        bytes32 key = keccak256(abi.encode(token, denom, commitment));
        if (commitmentUsed[key]) revert Rejected();
        commitmentUsed[key] = true;
        _useCap(depositCap[token], depositWindow[token], denom);
    }

    function _recordDeposit(address token, uint256 denom, bytes32 commitment) internal {
        emit Deposit(token, denom, commitment, depositCount++);
    }

    // --------------------------------------------------------------- outbound
    /// LEZ burn relay (permissionless): one proof per batch, covering the contiguous range of
    /// LEZ blocks after `lezTip` up to a final block. Constant gas whatever the range length.
    /// journal = abi.encode(cfg, prevRoot, newRoot, prevTip, newTip, newHeight, burns)
    function relay(bytes calldata seal, bytes calldata journal) external {
        (bytes32 jcfg, bytes32 prevRoot, bytes32 newRoot, bytes32 prevTip, bytes32 newTip, uint64 newHeight, uint64 burns) =
            abi.decode(journal, (bytes32, bytes32, bytes32, bytes32, bytes32, uint64, uint64));
        if (block.chainid != chainId || jcfg != cfg || prevRoot != latestBurnRoot || prevTip != lezTip || newHeight <= lezTipHeight) revert Rejected();
        if (!finality.isFinalLezBlock(newTip)) revert Rejected(); // final tip => final ancestors
        verifier.verify(seal, relayImageId, sha256(journal));
        burnRoot[newRoot] = true;
        latestBurnRoot = newRoot;
        lezTip = newTip;
        lezTipHeight = newHeight;
        emit BurnRootAccepted(newRoot, newTip, newHeight, burns);
    }

    struct ReleaseJournal {
        bytes32 cfg;
        bytes32 root;
        bytes32 nullifier;
        address dest;
        uint8 asNative; // 1: pay native ETH (WETH token only)
        address token;
        uint256 denom;
        address relayer;
        uint256 relayerFee;
    }

    /// F.7, Sec.3: every field acted on is in the proved journal; the vault pays nothing else.
    function release(bytes calldata seal, bytes calldata journal) external {
        ReleaseJournal memory j = abi.decode(journal, (ReleaseJournal));
        if (block.chainid != chainId || j.cfg != cfg || !burnRoot[j.root]) revert Rejected();
        if (burnNullifierSpent[j.nullifier]) revert Rejected();
        if (releasesFrozen || tokenFrozen[j.token]) revert Frozen();
        if (!permitted[j.token][j.denom] || !_feeAccepted(j.token, j.denom, j.relayerFee)) revert Unsupported();
        _useCap(releaseCap[j.token], releaseWindow[j.token], j.denom);
        verifier.verify(seal, releaseImageId, sha256(journal));
        burnNullifierSpent[j.nullifier] = true;

        uint256 toDest = j.denom - j.relayerFee;
        if (j.asNative == 1 && j.token == address(weth)) {
            weth.withdraw(j.denom);
            (bool ok,) = j.dest.call{value: toDest}("");
            if (!ok) revert Rejected();
            if (j.relayerFee > 0) {
                (ok,) = j.relayer.call{value: j.relayerFee}("");
                if (!ok) revert Rejected();
            }
        } else {
            IERC20(j.token).safeTransfer(j.dest, toDest);
            if (j.relayerFee > 0) IERC20(j.token).safeTransfer(j.relayer, j.relayerFee);
        }
        emit Released(j.token, j.denom);
    }

    function _feeAccepted(address token, uint256 denom, uint256 fee) internal view returns (bool) {
        if (fee == relayerFee[token][denom]) return true;
        return fee == prevRelayerFee[token][denom] && block.timestamp < prevRelayerFeeUntil[token][denom];
    }

    // ------------------------------------------------------------------- caps
    function _useCap(uint256 cap, Window storage w, uint256 amount) internal {
        if (cap == 0) return; // 0 = no cap configured
        Window memory m = w;
        uint256 elapsed = block.timestamp - m.start;
        if (elapsed >= 2 * WINDOW) {
            (m.start, m.prev, m.cur) = (uint32(block.timestamp), 0, 0);
            elapsed = 0;
        } else if (elapsed >= WINDOW) {
            (m.start, m.prev, m.cur) = (m.start + uint32(WINDOW), m.cur, 0);
            elapsed -= WINDOW;
        }
        uint256 used = (uint256(m.prev) * (WINDOW - elapsed)) / WINDOW + m.cur + amount;
        if (used > cap) revert CapExceeded();
        m.cur += uint112(amount);
        w.start = m.start;
        w.prev = m.prev;
        w.cur = m.cur;
    }
}
