// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title L1DepositInbox
/// @notice Canonical L1 entry point for funds that should be credited on L2.
contract L1DepositInbox {
    error InvalidRecipient();
    error EmptyDeposit();

    event DepositAccepted(
        uint256 indexed depositId,
        address indexed depositor,
        address indexed l2Recipient,
        uint256 amount,
        bytes32 previousDepositCommitment,
        bytes32 rollingDepositCommitment
    );

    uint256 public nextDepositId;
    uint256 public totalDeposited;
    bytes32 public rollingDepositCommitment;

    function deposit(address l2Recipient) external payable returns (uint256 depositId) {
        if (l2Recipient == address(0)) revert InvalidRecipient();
        if (msg.value == 0) revert EmptyDeposit();

        depositId = nextDepositId;
        bytes32 previous = rollingDepositCommitment;
        bytes32 nextCommitment = keccak256(abi.encode(previous, depositId, msg.sender, l2Recipient, msg.value));

        nextDepositId = depositId + 1;
        totalDeposited += msg.value;
        rollingDepositCommitment = nextCommitment;

        emit DepositAccepted(depositId, msg.sender, l2Recipient, msg.value, previous, nextCommitment);
    }
}
