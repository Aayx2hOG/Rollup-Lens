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

    event SequencerUpdated(address indexed sequencer);
    event BatchFinalized(
        uint256 indexed batchNumber,
        bytes32 indexed prevStateRoot,
        bytes32 indexed newStateRoot,
        bytes32 transactionRoot,
        bytes32 blockHashRoot,
        bytes32 dataHash,
        uint256 transactionCount,
        bytes32 batchCommitment
    );

    address public immutable owner;
    ISP1Verifier public immutable sp1Verifier;
    bytes32 public immutable programVKey;

    address public sequencer;
    bytes32 public finalizedStateRoot;
    uint256 public nextBatchNumber;

    /// @notice keccak256 of the complete 224-byte SP1 public-values payload.
    mapping(uint256 => bytes32) public batchCommitments;

    modifier onlyOwner() {
        if (msg.sender != owner) revert NotOwner();
        _;
    }

    modifier onlySequencer() {
        if (msg.sender != sequencer) revert NotSequencer();
        _;
    }

    constructor(bytes32 genesisStateRoot, ISP1Verifier sp1Verifier_, bytes32 programVKey_) {
        if (genesisStateRoot == bytes32(0)) revert InvalidRoot();
        if (address(sp1Verifier_) == address(0) || programVKey_ == bytes32(0)) {
            revert InvalidVerifier();
        }

        owner = msg.sender;
        sp1Verifier = sp1Verifier_;
        programVKey = programVKey_;
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
        if (publicValues.length != 224) revert InvalidPublicValues();
        if (
            keccak256(publicValues[:192])
                != keccak256(
                    abi.encode(batchNumber, prevStateRoot, newStateRoot, transactionRoot, blockHashRoot, dataHash)
                )
        ) {
            revert InvalidPublicValues();
        }

        sp1Verifier.verifyProof(programVKey, publicValues, proof);

        uint256 transactionCount = uint256(bytes32(publicValues[192:224]));
        bytes32 batchCommitment = keccak256(publicValues);
        batchCommitments[batchNumber] = batchCommitment;
        finalizedStateRoot = newStateRoot;
        nextBatchNumber = batchNumber + 1;

        emit BatchFinalized(
            batchNumber,
            prevStateRoot,
            newStateRoot,
            transactionRoot,
            blockHashRoot,
            dataHash,
            transactionCount,
            batchCommitment
        );
    }
}
