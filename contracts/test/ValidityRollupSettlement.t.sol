// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "../ValidityRollupSettlement.sol";

interface ValidityVm {
    function expectRevert(bytes4 selector) external;
    function prank(address sender) external;
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
    bytes32 private constant GENESIS_ROOT = bytes32(uint256(1));
    bytes32 private constant NEW_ROOT = bytes32(uint256(2));
    bytes32 private constant NEXT_ROOT = bytes32(uint256(3));
    bytes32 private constant TRANSACTION_ROOT = bytes32(uint256(4));
    bytes32 private constant BLOCK_HASH_ROOT = bytes32(uint256(5));
    bytes32 private constant DATA_HASH = bytes32(uint256(6));
    bytes32 private constant PROGRAM_VKEY = bytes32(uint256(7));

    ValidityRollupSettlement private settlement;

    function setUp() public {
        ValidityVerifierMock verifier = new ValidityVerifierMock(PROGRAM_VKEY);
        settlement = new ValidityRollupSettlement(GENESIS_ROOT, verifier, PROGRAM_VKEY);
        settlement.setSequencer(SEQUENCER);
    }

    function testProvenBatchStoresOnlyCommitmentAndLatestState() public {
        bytes memory values = publicValues(0, GENESIS_ROOT, NEW_ROOT, 12);
        submit(0, GENESIS_ROOT, NEW_ROOT, values, "proof");

        assertEq(settlement.nextBatchNumber(), 1, "next batch");
        assertEq(settlement.finalizedStateRoot(), NEW_ROOT, "latest root");
        assertEq(settlement.batchCommitments(0), keccak256(values), "batch commitment");
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
        return
            abi.encode(batchNumber, prevRoot, newRoot, TRANSACTION_ROOT, BLOCK_HASH_ROOT, DATA_HASH, transactionCount);
    }

    function assertEq(uint256 actual, uint256 expected, string memory label) private pure {
        require(actual == expected, label);
    }

    function assertEq(bytes32 actual, bytes32 expected, string memory label) private pure {
        require(actual == expected, label);
    }
}
