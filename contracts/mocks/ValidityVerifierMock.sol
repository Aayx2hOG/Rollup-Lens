// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ISP1Verifier} from "../ISP1Verifier.sol";

/// @notice Deterministic verifier for cross-system integration tests.
/// @dev This does not provide cryptographic security. ValidityRollupSettlement
/// still binds public values to the submitted batch before calling this mock.
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
