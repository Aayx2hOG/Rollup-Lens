// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "../ValidityRollupSettlement.sol";

interface ValidityVm {
    function expectRevert(bytes4 selector) external;
    function prank(address sender) external;
    function deal(address account, uint256 balance) external;
}

contract ValidityVerifierMock is ISP1Verifier {
    error InvalidProof();

    bytes32 private immutable expectedVKey;

    constructor(bytes32 expectedVKey_) {
        expectedVKey = expectedVKey_;
    }

    function verifyProof(bytes32 programVKey, bytes calldata, bytes calldata proof) external view {
        if (programVKey != expectedVKey || keccak256(proof) != keccak256("proof")) {
            revert InvalidProof();
        }
    }
}

contract ValidityRollupSettlementTest {
    ValidityVm private constant vm = ValidityVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    address private constant SEQUENCER = address(0xBEEF);
    address private constant L2_SENDER = address(0xCAFE);
    address payable private constant L1_RECIPIENT = payable(address(0xABCD));
    bytes32 private constant GENESIS_ROOT = bytes32(uint256(1));
    bytes32 private constant NEW_ROOT = bytes32(uint256(2));
    bytes32 private constant NEXT_ROOT = bytes32(uint256(3));
    bytes32 private constant TRANSACTION_ROOT = bytes32(uint256(4));
    bytes32 private constant BLOCK_HASH_ROOT = bytes32(uint256(5));
    bytes32 private constant DATA_HASH = bytes32(uint256(6));
    bytes32 private constant PROGRAM_VKEY = bytes32(uint256(7));
    bytes32 private constant DEPOSIT_ROOT = bytes32(uint256(8));
    bytes32 private constant WITHDRAWAL_ROOT = bytes32(uint256(9));
    address private constant L1_DEPOSIT_INBOX = address(0x2222222222222222222222222222222222222222);

    ValidityRollupSettlement private settlement;

    function setUp() public {
        ValidityVerifierMock verifier = new ValidityVerifierMock(PROGRAM_VKEY);
        settlement = new ValidityRollupSettlement(GENESIS_ROOT, verifier, PROGRAM_VKEY, L1_DEPOSIT_INBOX);
        settlement.setSequencer(SEQUENCER);
    }

    function testProvenBatchStoresOnlyCommitmentAndLatestState() public {
        bytes memory values = publicValues(0, GENESIS_ROOT, NEW_ROOT, 12);
        submit(0, GENESIS_ROOT, NEW_ROOT, values, "proof");

        assertEq(settlement.nextBatchNumber(), 1, "next batch");
        assertEq(settlement.finalizedStateRoot(), NEW_ROOT, "latest root");
        assertEq(settlement.batchCommitments(0), keccak256(values), "batch commitment");
        assertEq(settlement.withdrawalRoots(0), WITHDRAWAL_ROOT, "withdrawal root");
    }

    function testSequentialProvenBatchesUpdateLatestState() public {
        submit(0, GENESIS_ROOT, NEW_ROOT, publicValues(0, GENESIS_ROOT, NEW_ROOT, 12), "proof");
        submit(1, NEW_ROOT, NEXT_ROOT, publicValues(1, NEW_ROOT, NEXT_ROOT, 8), "proof");

        assertEq(settlement.nextBatchNumber(), 2, "next batch");
        assertEq(settlement.finalizedStateRoot(), NEXT_ROOT, "latest root");
    }

    function testProofCannotAuthorizeDifferentArguments() public {
        vm.prank(SEQUENCER);
        vm.expectRevert(ValidityRollupSettlement.InvalidPublicValues.selector);
        settlement.submitBatchWithProof(
            0,
            GENESIS_ROOT,
            NEXT_ROOT,
            TRANSACTION_ROOT,
            BLOCK_HASH_ROOT,
            DATA_HASH,
            publicValues(0, GENESIS_ROOT, NEW_ROOT, 12),
            "proof"
        );
    }

    function testInvalidProofIsRejectedWithoutWritingState() public {
        vm.prank(SEQUENCER);
        vm.expectRevert(ValidityVerifierMock.InvalidProof.selector);
        settlement.submitBatchWithProof(
            0,
            GENESIS_ROOT,
            NEW_ROOT,
            TRANSACTION_ROOT,
            BLOCK_HASH_ROOT,
            DATA_HASH,
            publicValues(0, GENESIS_ROOT, NEW_ROOT, 12),
            "invalid"
        );

        assertEq(settlement.nextBatchNumber(), 0, "next batch unchanged");
        assertEq(settlement.finalizedStateRoot(), GENESIS_ROOT, "root unchanged");
    }

    function testProofCannotAuthorizeDifferentDepositInbox() public {
        vm.prank(SEQUENCER);
        vm.expectRevert(ValidityRollupSettlement.InvalidPublicValues.selector);
        settlement.submitBatchWithProof(
            0,
            GENESIS_ROOT,
            NEW_ROOT,
            TRANSACTION_ROOT,
            BLOCK_HASH_ROOT,
            DATA_HASH,
            abi.encode(
                uint256(0),
                GENESIS_ROOT,
                NEW_ROOT,
                TRANSACTION_ROOT,
                BLOCK_HASH_ROOT,
                DATA_HASH,
                DEPOSIT_ROOT,
                uint256(0),
                uint256(0),
                uint256(1),
                uint256(12),
                WITHDRAWAL_ROOT,
                uint256(0),
                address(0x3333333333333333333333333333333333333333)
            ),
            "proof"
        );
    }

    function testInvalidDepositRangeIsRejected() public {
        vm.prank(SEQUENCER);
        vm.expectRevert(ValidityRollupSettlement.InvalidPublicValues.selector);
        settlement.submitBatchWithProof(
            0,
            GENESIS_ROOT,
            NEW_ROOT,
            TRANSACTION_ROOT,
            BLOCK_HASH_ROOT,
            DATA_HASH,
            abi.encode(
                uint256(0),
                GENESIS_ROOT,
                NEW_ROOT,
                TRANSACTION_ROOT,
                BLOCK_HASH_ROOT,
                DATA_HASH,
                DEPOSIT_ROOT,
                uint256(5),
                uint256(4),
                uint256(1),
                uint256(12),
                WITHDRAWAL_ROOT,
                uint256(0),
                L1_DEPOSIT_INBOX
            ),
            "proof"
        );
    }

    function testClaimWithdrawalFromProvenRoot() public {
        bytes32 withdrawalHash = bytes32(uint256(0x1234));
        uint256 amount = 1 ether;
        uint256 nonce = 4;
        bytes32 leaf = sha256(
            abi.encodePacked(
                "zk-rollup-withdrawal-leaf-v1",
                withdrawalHash,
                L2_SENDER,
                L1_RECIPIENT,
                amount,
                nonce
            )
        );
        submit(0, GENESIS_ROOT, NEW_ROOT, publicValuesWithWithdrawalRoot(0, GENESIS_ROOT, NEW_ROOT, 12, leaf, 1), "proof");
        vm.deal(address(settlement), amount);

        uint256 beforeBalance = L1_RECIPIENT.balance;
        settlement.claimWithdrawal(
            0,
            0,
            withdrawalHash,
            L2_SENDER,
            L1_RECIPIENT,
            amount,
            nonce,
            new bytes32[](0),
            new bool[](0)
        );

        assertEq(L1_RECIPIENT.balance, beforeBalance + amount, "recipient paid");
        bytes32 withdrawalId = keccak256(abi.encode(uint256(0), uint256(0), leaf));
        require(settlement.claimedWithdrawals(withdrawalId), "claimed");
    }

    function submit(uint256 batchNumber, bytes32 prevRoot, bytes32 newRoot, bytes memory values, bytes memory proof)
        private
    {
        vm.prank(SEQUENCER);
        settlement.submitBatchWithProof(
            batchNumber, prevRoot, newRoot, TRANSACTION_ROOT, BLOCK_HASH_ROOT, DATA_HASH, values, proof
        );
    }

    function publicValues(uint256 batchNumber, bytes32 prevRoot, bytes32 newRoot, uint256 transactionCount)
        private
        pure
        returns (bytes memory)
    {
        return publicValuesWithWithdrawalRoot(batchNumber, prevRoot, newRoot, transactionCount, WITHDRAWAL_ROOT, 0);
    }

    function publicValuesWithWithdrawalRoot(
        uint256 batchNumber,
        bytes32 prevRoot,
        bytes32 newRoot,
        uint256 transactionCount,
        bytes32 withdrawalRoot,
        uint256 withdrawalCount
    ) private pure returns (bytes memory) {
        return abi.encode(
            batchNumber,
            prevRoot,
            newRoot,
            TRANSACTION_ROOT,
            BLOCK_HASH_ROOT,
            DATA_HASH,
            DEPOSIT_ROOT,
            uint256(0),
            uint256(0),
            uint256(0),
            transactionCount,
            withdrawalRoot,
            withdrawalCount,
            L1_DEPOSIT_INBOX
        );
    }

    function assertEq(uint256 actual, uint256 expected, string memory label) private pure {
        require(actual == expected, label);
    }

    function assertEq(bytes32 actual, bytes32 expected, string memory label) private pure {
        require(actual == expected, label);
    }
}
