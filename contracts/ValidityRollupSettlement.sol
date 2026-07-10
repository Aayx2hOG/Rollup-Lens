// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ISP1Verifier} from "./ISP1Verifier.sol";

/// @title ValidityRollupSettlement
/// @notice Minimal settlement contract for batches proven by an SP1 program.
/// @dev Historical batch data is emitted in events. Only one commitment per
///      batch and the latest finalized state are retained in contract storage.
contract ValidityRollupSettlement {
    error NotOwner();
    error NotSequencer();
    error InvalidVerifier();
    error InvalidRoot();
    error InvalidBatchNumber();
    error InvalidStateTransition();
    error InvalidPublicValues();
    error InvalidWithdrawalProof();
    error WithdrawalAlreadyClaimed();
    error WithdrawalTransferFailed();

    event SequencerUpdated(address indexed sequencer);
    event BatchFinalized(
        uint256 indexed batchNumber,
        bytes32 indexed prevStateRoot,
        bytes32 indexed newStateRoot,
        bytes32 transactionRoot,
        bytes32 blockHashRoot,
        bytes32 dataHash,
        bytes32 depositRoot,
        uint256 firstDepositId,
        uint256 lastDepositId,
        uint256 depositCount,
        uint256 transactionCount,
        bytes32 withdrawalRoot,
        uint256 withdrawalCount,
        address l1DepositInbox,
        bytes32 batchCommitment
    );

    event WithdrawalClaimed(
        uint256 indexed batchNumber,
        bytes32 indexed withdrawalId,
        address indexed l1Recipient,
        uint256 amount
    );

    address public immutable owner;
    ISP1Verifier public immutable sp1Verifier;
    bytes32 public immutable programVKey;
    address public immutable l1DepositInbox;

    address public sequencer;
    bytes32 public finalizedStateRoot;
    uint256 public nextBatchNumber;

    /// @notice keccak256 of the complete 448-byte SP1 public-values payload.
    mapping(uint256 => bytes32) public batchCommitments;
    mapping(uint256 => bytes32) public withdrawalRoots;
    mapping(bytes32 => bool) public claimedWithdrawals;

    modifier onlyOwner() {
        if (msg.sender != owner) revert NotOwner();
        _;
    }

    modifier onlySequencer() {
        if (msg.sender != sequencer) revert NotSequencer();
        _;
    }

    constructor(bytes32 genesisStateRoot, ISP1Verifier sp1Verifier_, bytes32 programVKey_, address l1DepositInbox_) {
        if (genesisStateRoot == bytes32(0)) revert InvalidRoot();
        if (address(sp1Verifier_) == address(0) || programVKey_ == bytes32(0) || l1DepositInbox_ == address(0)) {
            revert InvalidVerifier();
        }

        owner = msg.sender;
        sp1Verifier = sp1Verifier_;
        programVKey = programVKey_;
        l1DepositInbox = l1DepositInbox_;
        finalizedStateRoot = genesisStateRoot;
    }

    function setSequencer(address newSequencer) external onlyOwner {
        if (newSequencer == address(0)) revert NotSequencer();
        sequencer = newSequencer;
        emit SequencerUpdated(newSequencer);
    }

    function submitBatchWithProof(
        uint256 batchNumber,
        bytes32 prevStateRoot,
        bytes32 newStateRoot,
        bytes32 transactionRoot,
        bytes32 blockHashRoot,
        bytes32 dataHash,
        bytes calldata publicValues,
        bytes calldata proof
    ) external onlySequencer {
        if (batchNumber != nextBatchNumber) revert InvalidBatchNumber();
        if (prevStateRoot != finalizedStateRoot) revert InvalidStateTransition();
        if (
            newStateRoot == bytes32(0) || transactionRoot == bytes32(0) || blockHashRoot == bytes32(0)
                || dataHash == bytes32(0)
        ) {
            revert InvalidRoot();
        }
        if (publicValues.length != 448) revert InvalidPublicValues();
        if (
            keccak256(publicValues[:192])
                != keccak256(
                    abi.encode(batchNumber, prevStateRoot, newStateRoot, transactionRoot, blockHashRoot, dataHash)
                )
        ) {
            revert InvalidPublicValues();
        }
        bytes32 depositRoot = bytes32(publicValues[192:224]);
        uint256 firstDepositId = uint256(bytes32(publicValues[224:256]));
        uint256 lastDepositId = uint256(bytes32(publicValues[256:288]));
        uint256 depositCount = uint256(bytes32(publicValues[288:320]));
        uint256 transactionCount = uint256(bytes32(publicValues[320:352]));
        bytes32 withdrawalRoot = bytes32(publicValues[352:384]);
        uint256 withdrawalCount = uint256(bytes32(publicValues[384:416]));
        address publicInbox = address(uint160(uint256(bytes32(publicValues[416:448]))));
        if (publicInbox != l1DepositInbox) revert InvalidPublicValues();
        if (depositCount == 0) {
            if (firstDepositId != 0 || lastDepositId != 0) revert InvalidPublicValues();
        } else if (lastDepositId < firstDepositId) {
            revert InvalidPublicValues();
        }

        sp1Verifier.verifyProof(programVKey, publicValues, proof);

        bytes32 batchCommitment = keccak256(publicValues);
        batchCommitments[batchNumber] = batchCommitment;
        withdrawalRoots[batchNumber] = withdrawalRoot;
        finalizedStateRoot = newStateRoot;
        nextBatchNumber = batchNumber + 1;

        emit BatchFinalized(
            batchNumber,
            prevStateRoot,
            newStateRoot,
            transactionRoot,
            blockHashRoot,
            dataHash,
            depositRoot,
            firstDepositId,
            lastDepositId,
            depositCount,
            transactionCount,
            withdrawalRoot,
            withdrawalCount,
            publicInbox,
            batchCommitment
        );
    }

    function claimWithdrawal(
        uint256 batchNumber,
        uint256 withdrawalIndex,
        bytes32 withdrawalHash,
        address l2Sender,
        address payable l1Recipient,
        uint256 amount,
        uint256 nonce,
        bytes32[] calldata proof,
        bool[] calldata siblingIsLeft
    ) external {
        bytes32 root = withdrawalRoots[batchNumber];
        if (root == bytes32(0)) revert InvalidWithdrawalProof();
        if (proof.length != siblingIsLeft.length) revert InvalidWithdrawalProof();
        bytes32 leafHash = sha256(
            abi.encodePacked(
                "zk-rollup-withdrawal-leaf-v1",
                withdrawalHash,
                l2Sender,
                l1Recipient,
                amount,
                nonce
            )
        );
        if (!_verifyWithdrawalProof(root, leafHash, proof, siblingIsLeft)) {
            revert InvalidWithdrawalProof();
        }

        bytes32 withdrawalId = keccak256(abi.encode(batchNumber, withdrawalIndex, leafHash));
        if (claimedWithdrawals[withdrawalId]) revert WithdrawalAlreadyClaimed();
        claimedWithdrawals[withdrawalId] = true;

        (bool ok,) = l1Recipient.call{value: amount}("");
        if (!ok) revert WithdrawalTransferFailed();
        emit WithdrawalClaimed(batchNumber, withdrawalId, l1Recipient, amount);
    }

    function _verifyWithdrawalProof(
        bytes32 root,
        bytes32 leafHash,
        bytes32[] calldata proof,
        bool[] calldata siblingIsLeft
    ) private pure returns (bool) {
        bytes32 computed = leafHash;
        for (uint256 i = 0; i < proof.length; i++) {
            computed = siblingIsLeft[i]
                ? sha256(abi.encodePacked(bytes1(0x01), proof[i], computed))
                : sha256(abi.encodePacked(bytes1(0x01), computed, proof[i]));
        }
        return computed == root;
    }

    receive() external payable {}
}
