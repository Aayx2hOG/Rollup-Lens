// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @notice Interface implemented by the official version-matched SP1 verifier.
interface ISP1Verifier {
    function verifyProof(bytes32 programVKey, bytes calldata publicValues, bytes calldata proofBytes) external view;
}
