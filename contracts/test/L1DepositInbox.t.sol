// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "../L1DepositInbox.sol";

interface DepositVm {
    function deal(address account, uint256 balance) external;
    function expectRevert(bytes4 selector) external;
    function prank(address sender) external;
}

contract L1DepositInboxTest {
    DepositVm private constant vm = DepositVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    address private constant ALICE = address(0xA11CE);
    address private constant BOB = address(0xB0B);

    L1DepositInbox private inbox;

    function setUp() public {
        inbox = new L1DepositInbox();
        vm.deal(ALICE, 10 ether);
    }

    function testDepositsUseSequentialIdsAndRollingCommitment() public {
        vm.prank(ALICE);
        uint256 firstId = inbox.deposit{value: 1 ether}(BOB);
        bytes32 firstCommitment = keccak256(abi.encode(bytes32(0), uint256(0), ALICE, BOB, uint256(1 ether)));

        assertEq(firstId, 0, "first id");
        assertEq(inbox.nextDepositId(), 1, "next id after first");
        assertEq(inbox.totalDeposited(), 1 ether, "total after first");
        assertEq(inbox.rollingDepositCommitment(), firstCommitment, "first commitment");

        vm.prank(ALICE);
        uint256 secondId = inbox.deposit{value: 2 ether}(BOB);
        bytes32 secondCommitment = keccak256(abi.encode(firstCommitment, uint256(1), ALICE, BOB, uint256(2 ether)));

        assertEq(secondId, 1, "second id");
        assertEq(inbox.nextDepositId(), 2, "next id after second");
        assertEq(inbox.totalDeposited(), 3 ether, "total after second");
        assertEq(inbox.rollingDepositCommitment(), secondCommitment, "second commitment");
    }

    function testRejectsEmptyDeposit() public {
        vm.prank(ALICE);
        vm.expectRevert(L1DepositInbox.EmptyDeposit.selector);
        inbox.deposit{value: 0}(BOB);
    }

    function testRejectsZeroRecipient() public {
        vm.prank(ALICE);
        vm.expectRevert(L1DepositInbox.InvalidRecipient.selector);
        inbox.deposit{value: 1 ether}(address(0));
    }

    function assertEq(uint256 actual, uint256 expected, string memory label) private pure {
        require(actual == expected, label);
    }

    function assertEq(bytes32 actual, bytes32 expected, string memory label) private pure {
        require(actual == expected, label);
    }
}
