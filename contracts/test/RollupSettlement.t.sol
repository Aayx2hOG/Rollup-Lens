// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "../RollupSettlement.sol";

interface Vm {
    function deal(address account, uint256 balance) external;
    function expectRevert(bytes4 selector) external;
    function prank(address sender) external;
    function warp(uint256 timestamp) external;
}

contract RollupSettlementTest {
    Vm private constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    address private constant SEQUENCER = address(0xBEEF);
    address private constant CHALLENGER = address(0xCAFE);

    bytes32 private constant GENESIS_ROOT = bytes32(uint256(1));
    bytes32 private constant NEW_ROOT = bytes32(uint256(2));
    bytes32 private constant REPLACEMENT_ROOT = bytes32(uint256(3));
    bytes32 private constant TRANSACTION_ROOT = bytes32(uint256(4));
    bytes32 private constant BLOCK_HASH_ROOT = bytes32(uint256(5));
    bytes32 private constant DATA_HASH = bytes32(uint256(6));

    uint256 private constant CHALLENGE_WINDOW = 1 days;
    uint256 private constant WITHDRAWAL_DELAY = 7 days;
    uint256 private constant MINIMUM_SEQUENCER_BOND = 10 ether;
    uint256 private constant CHALLENGE_BOND = 1 ether;

    RollupSettlement private settlement;

    receive() external payable {}

    function setUp() public {
        settlement = new RollupSettlement(
            GENESIS_ROOT,
            CHALLENGE_WINDOW,
            WITHDRAWAL_DELAY,
            MINIMUM_SEQUENCER_BOND,
            CHALLENGE_BOND
        );
        settlement.setSequencer(SEQUENCER);
    }

    function testSequencerCanSubmitAndFinalizeBatch() public {
        bondSequencer();
        submitBatch(0, GENESIS_ROOT, NEW_ROOT);

        assertEq(settlement.nextBatchNumber(), 1, "next batch number");
        assertTrue(settlement.hasPendingBatch(), "pending batch");

        vm.expectRevert(RollupSettlement.ChallengeWindowOpen.selector);
        settlement.finalizeBatch(0);

        vm.warp(block.timestamp + CHALLENGE_WINDOW);
        settlement.finalizeBatch(0);

        assertEq(settlement.finalizedBatchNumber(), 0, "finalized batch");
        assertEq(settlement.finalizedStateRoot(), NEW_ROOT, "finalized root");
        assertFalse(settlement.hasPendingBatch(), "pending batch cleared");
    }

    function testSequencerNeedsMinimumBondToSubmit() public {
        vm.prank(SEQUENCER);
        vm.expectRevert(RollupSettlement.InvalidBond.selector);
        settlement.submitBatch(
            0,
            GENESIS_ROOT,
            NEW_ROOT,
            TRANSACTION_ROOT,
            BLOCK_HASH_ROOT,
            DATA_HASH
        );
    }

    function testChallengeBlocksFinalizationAndRejectedBatchCanBeReplaced() public {
        bondSequencer();
        submitBatch(0, GENESIS_ROOT, NEW_ROOT);

        vm.deal(CHALLENGER, CHALLENGE_BOND);
        vm.prank(CHALLENGER);
        settlement.challengeBatch{value: CHALLENGE_BOND}(0, "invalid transition");

        vm.warp(block.timestamp + CHALLENGE_WINDOW);
        vm.expectRevert(RollupSettlement.BatchAlreadyChallenged.selector);
        settlement.finalizeBatch(0);

        settlement.rejectChallengedBatch(0);
        assertEq(settlement.nextBatchNumber(), 0, "reopened batch number");
        assertFalse(settlement.hasPendingBatch(), "rejected batch cleared");

        bondSequencer();
        submitBatch(0, GENESIS_ROOT, REPLACEMENT_ROOT);

        vm.warp(block.timestamp + CHALLENGE_WINDOW);
        settlement.finalizeBatch(0);
        assertEq(settlement.finalizedStateRoot(), REPLACEMENT_ROOT, "replacement finalized");
    }

    function bondSequencer() private {
        vm.deal(SEQUENCER, MINIMUM_SEQUENCER_BOND);
        vm.prank(SEQUENCER);
        settlement.depositSequencerBond{value: MINIMUM_SEQUENCER_BOND}();
    }

    function submitBatch(uint256 batchNumber, bytes32 prevRoot, bytes32 newRoot) private {
        vm.prank(SEQUENCER);
        settlement.submitBatch(
            batchNumber,
            prevRoot,
            newRoot,
            TRANSACTION_ROOT,
            BLOCK_HASH_ROOT,
            DATA_HASH
        );
    }

    function assertEq(uint256 actual, uint256 expected, string memory label) private pure {
        require(actual == expected, label);
    }

    function assertEq(bytes32 actual, bytes32 expected, string memory label) private pure {
        require(actual == expected, label);
    }

    function assertTrue(bool value, string memory label) private pure {
        require(value, label);
    }

    function assertFalse(bool value, string memory label) private pure {
        require(!value, label);
    }
}
