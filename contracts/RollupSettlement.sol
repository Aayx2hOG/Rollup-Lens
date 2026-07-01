// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ISP1Verifier} from "./ISP1Verifier.sol";

/// @title RollupSettlement
/// @notice Optimistic settlement contract for the local Rust rollup simulator.
/// @dev Batches are accepted only after a challenge window. A challenged batch
///      blocks finalization until governance or a future fraud-proof verifier
///      resolves it.
contract RollupSettlement {
    struct Batch {
        bytes32 prevStateRoot;
        bytes32 newStateRoot;
        bytes32 transactionRoot;
        bytes32 blockHashRoot;
        bytes32 dataHash;
        uint256 submittedAt;
        uint256 finalizedAt;
        address sequencer;
        address challenger;
        bool challenged;
        bool finalized;
        bool rejected;
    }

    struct ForcedWithdrawal {
        address requester;
        bytes32 l2Account;
        uint256 amount;
        uint256 requestedAt;
        bool resolved;
    }

    error NotOwner();
    error NotSequencer();
    error InvalidBond();
    error InvalidRoot();
    error InvalidWindow();
    error InvalidBatchNumber();
    error InvalidStateTransition();
    error PendingBatchExists();
    error ChallengeWindowOpen();
    error ChallengeWindowClosed();
    error BatchAlreadyChallenged();
    error BatchNotChallenged();
    error BatchAlreadyFinalized();
    error BatchAlreadyRejected();
    error BatchNotFinalized();
    error WithdrawalAlreadyResolved();
    error WithdrawalDelayOpen();
    error InsufficientBond();
    error EmptyChallengeProof();
    error TransferFailed();
    error InvalidVerifier();
    error InvalidPublicValues();

    event SequencerUpdated(address indexed sequencer);
    event BatchSubmitted(
        uint256 indexed batchNumber,
        bytes32 indexed prevStateRoot,
        bytes32 indexed newStateRoot,
        bytes32 transactionRoot,
        bytes32 blockHashRoot,
        bytes32 dataHash,
        address sequencer
    );
    event BatchChallenged(uint256 indexed batchNumber, address indexed challenger, bytes proof);
    event BatchRejected(uint256 indexed batchNumber);
    event BatchChallengeDismissed(uint256 indexed batchNumber);
    event BatchFinalized(uint256 indexed batchNumber, bytes32 indexed stateRoot);
    event ValidityProofVerified(uint256 indexed batchNumber, bytes32 indexed programVKey);
    event ForcedWithdrawalRequested(
        uint256 indexed requestId, address indexed requester, bytes32 indexed l2Account, uint256 amount
    );
    event ForcedWithdrawalResolved(uint256 indexed requestId);

    address public immutable owner;
    uint256 public immutable challengeWindow;
    uint256 public immutable forcedWithdrawalDelay;
    uint256 public immutable minimumSequencerBond;
    uint256 public immutable challengeBond;
    ISP1Verifier public immutable sp1Verifier;
    bytes32 public immutable programVKey;

    address public sequencer;
    bytes32 public finalizedStateRoot;
    uint256 public finalizedBatchNumber;
    uint256 public nextBatchNumber;
    uint256 public nextWithdrawalRequestId;
    bool public hasPendingBatch;

    mapping(address => uint256) public sequencerBond;
    mapping(uint256 => Batch) public batches;
    mapping(uint256 => ForcedWithdrawal) public forcedWithdrawals;

    modifier onlyOwner() {
        if (msg.sender != owner) revert NotOwner();
        _;
    }

    modifier onlySequencer() {
        if (msg.sender != sequencer) revert NotSequencer();
        if (sequencerBond[msg.sender] < minimumSequencerBond) revert InvalidBond();
        _;
    }

    constructor(
        bytes32 genesisStateRoot,
        uint256 challengeWindow_,
        uint256 forcedWithdrawalDelay_,
        uint256 minimumSequencerBond_,
        uint256 challengeBond_,
        ISP1Verifier sp1Verifier_,
        bytes32 programVKey_
    ) {
        if (genesisStateRoot == bytes32(0)) revert InvalidRoot();
        if (challengeWindow_ == 0 || forcedWithdrawalDelay_ == 0) revert InvalidWindow();
        if (minimumSequencerBond_ == 0 || challengeBond_ == 0) revert InvalidBond();
        if (address(sp1Verifier_) == address(0) || programVKey_ == bytes32(0)) {
            revert InvalidVerifier();
        }

        owner = msg.sender;
        finalizedStateRoot = genesisStateRoot;
        challengeWindow = challengeWindow_;
        forcedWithdrawalDelay = forcedWithdrawalDelay_;
        minimumSequencerBond = minimumSequencerBond_;
        challengeBond = challengeBond_;
        sp1Verifier = sp1Verifier_;
        programVKey = programVKey_;
    }

    receive() external payable {
        sequencerBond[msg.sender] += msg.value;
    }

    function setSequencer(address newSequencer) external onlyOwner {
        if (newSequencer == address(0)) revert NotSequencer();
        sequencer = newSequencer;
        emit SequencerUpdated(newSequencer);
    }

    function depositSequencerBond() external payable {
        sequencerBond[msg.sender] += msg.value;
    }

    function withdrawSequencerBond(uint256 amount) external {
        if (sequencerBond[msg.sender] < amount) revert InsufficientBond();
        uint256 remaining = sequencerBond[msg.sender] - amount;
        if (msg.sender == sequencer && remaining < minimumSequencerBond) revert InvalidBond();

        sequencerBond[msg.sender] = remaining;
        (bool ok,) = msg.sender.call{value: amount}("");
        if (!ok) revert TransferFailed();
    }

    function submitBatch(
        uint256 batchNumber,
        bytes32 prevStateRoot,
        bytes32 newStateRoot,
        bytes32 transactionRoot,
        bytes32 blockHashRoot,
        bytes32 dataHash
    ) external onlySequencer {
        if (batchNumber != nextBatchNumber) revert InvalidBatchNumber();
        if (hasPendingBatch) revert PendingBatchExists();
        if (prevStateRoot != finalizedStateRoot) revert InvalidStateTransition();
        if (
            newStateRoot == bytes32(0) || transactionRoot == bytes32(0) || blockHashRoot == bytes32(0)
                || dataHash == bytes32(0)
        ) {
            revert InvalidRoot();
        }

        batches[batchNumber] = Batch({
            prevStateRoot: prevStateRoot,
            newStateRoot: newStateRoot,
            transactionRoot: transactionRoot,
            blockHashRoot: blockHashRoot,
            dataHash: dataHash,
            submittedAt: block.timestamp,
            finalizedAt: 0,
            sequencer: msg.sender,
            challenger: address(0),
            challenged: false,
            finalized: false,
            rejected: false
        });
        nextBatchNumber = batchNumber + 1;
        hasPendingBatch = true;

        emit BatchSubmitted(
            batchNumber, prevStateRoot, newStateRoot, transactionRoot, blockHashRoot, dataHash, msg.sender
        );
    }

    /// @notice Submit and immediately finalize a batch backed by an SP1 Groth16 proof.
    /// @dev `publicValues` must be the guest's ABI encoding of all batch commitments.
    ///      The official SP1 verifier reverts unless `proof` proves execution of the
    ///      immutable program verification key with exactly these public values.
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
        if (hasPendingBatch) revert PendingBatchExists();
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

        batches[batchNumber] = Batch({
            prevStateRoot: prevStateRoot,
            newStateRoot: newStateRoot,
            transactionRoot: transactionRoot,
            blockHashRoot: blockHashRoot,
            dataHash: dataHash,
            submittedAt: block.timestamp,
            finalizedAt: block.timestamp,
            sequencer: msg.sender,
            challenger: address(0),
            challenged: false,
            finalized: true,
            rejected: false
        });
        nextBatchNumber = batchNumber + 1;
        finalizedBatchNumber = batchNumber;
        finalizedStateRoot = newStateRoot;

        emit BatchSubmitted(
            batchNumber, prevStateRoot, newStateRoot, transactionRoot, blockHashRoot, dataHash, msg.sender
        );
        emit ValidityProofVerified(batchNumber, programVKey);
        emit BatchFinalized(batchNumber, newStateRoot);
    }

    function challengeBatch(uint256 batchNumber, bytes calldata proof) external payable {
        Batch storage batch = batches[batchNumber];
        if (batch.submittedAt == 0) revert InvalidBatchNumber();
        if (batch.finalized) revert BatchAlreadyFinalized();
        if (batch.rejected) revert BatchAlreadyRejected();
        if (batch.challenged) revert BatchAlreadyChallenged();
        if (proof.length == 0) revert EmptyChallengeProof();
        if (msg.value != challengeBond) revert InvalidBond();
        if (block.timestamp >= batch.submittedAt + challengeWindow) revert ChallengeWindowClosed();

        batch.challenged = true;
        batch.challenger = msg.sender;
        emit BatchChallenged(batchNumber, msg.sender, proof);
    }

    function rejectChallengedBatch(uint256 batchNumber) external onlyOwner {
        Batch storage batch = batches[batchNumber];
        if (!batch.challenged) revert BatchNotChallenged();
        if (batch.finalized) revert BatchAlreadyFinalized();

        batch.rejected = true;
        hasPendingBatch = false;
        nextBatchNumber = batchNumber;

        uint256 slashed = sequencerBond[batch.sequencer] > minimumSequencerBond
            ? minimumSequencerBond
            : sequencerBond[batch.sequencer];
        sequencerBond[batch.sequencer] -= slashed;

        uint256 payout = challengeBond + slashed;
        if (payout != 0) {
            (bool ok,) = batch.challenger.call{value: payout}("");
            if (!ok) revert TransferFailed();
        }

        emit BatchRejected(batchNumber);
    }

    function dismissChallenge(uint256 batchNumber) external onlyOwner {
        Batch storage batch = batches[batchNumber];
        if (!batch.challenged) revert BatchNotChallenged();
        if (batch.finalized) revert BatchAlreadyFinalized();
        if (batch.rejected) revert BatchAlreadyRejected();

        batch.challenged = false;
        batch.challenger = address(0);
        sequencerBond[batch.sequencer] += challengeBond;

        emit BatchChallengeDismissed(batchNumber);
    }

    function finalizeBatch(uint256 batchNumber) external {
        Batch storage batch = batches[batchNumber];
        if (batch.submittedAt == 0) revert InvalidBatchNumber();
        if (batch.finalized) revert BatchAlreadyFinalized();
        if (batch.rejected) revert BatchAlreadyRejected();
        if (batch.challenged) revert BatchAlreadyChallenged();
        if (block.timestamp < batch.submittedAt + challengeWindow) revert ChallengeWindowOpen();
        if (batch.prevStateRoot != finalizedStateRoot) revert InvalidStateTransition();

        batch.finalized = true;
        batch.finalizedAt = block.timestamp;
        finalizedStateRoot = batch.newStateRoot;
        finalizedBatchNumber = batchNumber;
        hasPendingBatch = false;

        emit BatchFinalized(batchNumber, batch.newStateRoot);
    }

    function requestForcedWithdrawal(bytes32 l2Account, uint256 amount) external returns (uint256) {
        if (l2Account == bytes32(0) || amount == 0) revert InvalidRoot();

        uint256 requestId = nextWithdrawalRequestId++;
        forcedWithdrawals[requestId] = ForcedWithdrawal({
            requester: msg.sender, l2Account: l2Account, amount: amount, requestedAt: block.timestamp, resolved: false
        });

        emit ForcedWithdrawalRequested(requestId, msg.sender, l2Account, amount);
        return requestId;
    }

    function markForcedWithdrawalResolved(uint256 requestId) external onlySequencer {
        ForcedWithdrawal storage request = forcedWithdrawals[requestId];
        if (request.resolved) revert WithdrawalAlreadyResolved();

        request.resolved = true;
        emit ForcedWithdrawalResolved(requestId);
    }

    function forceExitExpiredWithdrawal(uint256 requestId) external {
        ForcedWithdrawal storage request = forcedWithdrawals[requestId];
        if (request.resolved) revert WithdrawalAlreadyResolved();
        if (block.timestamp < request.requestedAt + forcedWithdrawalDelay) {
            revert WithdrawalDelayOpen();
        }

        request.resolved = true;
        emit ForcedWithdrawalResolved(requestId);
    }
}
