// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "../ValidityRollupSettlement.sol";
import {SP1Verifier as SP1Groth16Verifier} from "sp1-contracts/src/v6.1.0/SP1VerifierGroth16.sol";

interface SP1IntegrationVm {
    function envOr(string calldata name, bytes32 defaultValue) external view returns (bytes32);
    function envOr(string calldata name, bytes calldata defaultValue) external view returns (bytes memory);
    function expectRevert() external;
    function skip(bool skipTest) external;
}

/// @notice Opt-in integration tests against SP1's real v6.1.0 Groth16 verifier.
/// Set SP1_PROGRAM_VKEY, SP1_PUBLIC_VALUES, and SP1_PROOF to a proof artifact's
/// fields before running this contract. With no artifact, tests return early.
contract ValidityRollupSettlementSP1Test {
    SP1IntegrationVm private constant vm = SP1IntegrationVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function testRealSP1VerifierAcceptsGeneratedBatchProof() public {
        (bytes32 programVKey, bytes memory publicValues, bytes memory proof) = artifact();
        if (proof.length == 0) {
            vm.skip(true);
            return;
        }

        (
            uint256 batchNumber,
            bytes32 prevStateRoot,
            bytes32 newStateRoot,
            bytes32 transactionRoot,
            bytes32 blockHashRoot,
            bytes32 dataHash,
            uint256 transactionCount
        ) = abi.decode(publicValues, (uint256, bytes32, bytes32, bytes32, bytes32, bytes32, uint256));

        SP1Groth16Verifier verifier = new SP1Groth16Verifier();
        ValidityRollupSettlement settlement =
            new ValidityRollupSettlement(prevStateRoot, ISP1Verifier(address(verifier)), programVKey);
        settlement.setSequencer(address(this));
        settlement.submitBatchWithProof(
            batchNumber, prevStateRoot, newStateRoot, transactionRoot, blockHashRoot, dataHash, publicValues, proof
        );

        require(settlement.finalizedStateRoot() == newStateRoot, "finalized root");
        require(settlement.nextBatchNumber() == batchNumber + 1, "next batch");
        require(settlement.batchCommitments(batchNumber) == keccak256(publicValues), "commitment");
        require(transactionCount > 0, "fixture must exercise a transaction");
    }

    function testRealSP1VerifierRejectsTamperedPublicValues() public {
        (bytes32 programVKey, bytes memory publicValues, bytes memory proof) = artifact();
        if (proof.length == 0) {
            vm.skip(true);
            return;
        }

        (
            uint256 batchNumber,
            bytes32 prevStateRoot,
            bytes32 newStateRoot,
            bytes32 transactionRoot,
            bytes32 blockHashRoot,
            bytes32 dataHash,
        ) = abi.decode(publicValues, (uint256, bytes32, bytes32, bytes32, bytes32, bytes32, uint256));

        publicValues[63] ^= bytes1(uint8(1));
        assembly {
            newStateRoot := mload(add(publicValues, 64))
        }

        SP1Groth16Verifier verifier = new SP1Groth16Verifier();
        ValidityRollupSettlement settlement =
            new ValidityRollupSettlement(prevStateRoot, ISP1Verifier(address(verifier)), programVKey);
        settlement.setSequencer(address(this));

        vm.expectRevert();
        settlement.submitBatchWithProof(
            batchNumber, prevStateRoot, newStateRoot, transactionRoot, blockHashRoot, dataHash, publicValues, proof
        );
    }

    function testRealSP1VerifierRejectsTamperedProof() public {
        (bytes32 programVKey, bytes memory publicValues, bytes memory proof) = artifact();
        if (proof.length == 0) {
            vm.skip(true);
            return;
        }

        (
            uint256 batchNumber,
            bytes32 prevStateRoot,
            bytes32 newStateRoot,
            bytes32 transactionRoot,
            bytes32 blockHashRoot,
            bytes32 dataHash,
        ) = abi.decode(publicValues, (uint256, bytes32, bytes32, bytes32, bytes32, bytes32, uint256));

        proof[proof.length - 1] ^= bytes1(uint8(1));

        SP1Groth16Verifier verifier = new SP1Groth16Verifier();
        ValidityRollupSettlement settlement =
            new ValidityRollupSettlement(prevStateRoot, ISP1Verifier(address(verifier)), programVKey);
        settlement.setSequencer(address(this));

        vm.expectRevert();
        settlement.submitBatchWithProof(
            batchNumber, prevStateRoot, newStateRoot, transactionRoot, blockHashRoot, dataHash, publicValues, proof
        );
    }

    function artifact() private view returns (bytes32 programVKey, bytes memory publicValues, bytes memory proof) {
        programVKey = vm.envOr("SP1_PROGRAM_VKEY", bytes32(0));
        publicValues = vm.envOr("SP1_PUBLIC_VALUES", bytes(""));
        proof = vm.envOr("SP1_PROOF", bytes(""));
        if (proof.length != 0) {
            require(programVKey != bytes32(0), "missing SP1_PROGRAM_VKEY");
            require(publicValues.length == 224, "invalid SP1_PUBLIC_VALUES");
        }
    }
}
