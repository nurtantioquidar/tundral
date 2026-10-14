// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

// NOTE: Graphify has no Solidity grammar installed; these files count but add nothing.
abstract contract Base {
    address public owner;

    constructor() {
        owner = msg.sender;
    }

    function bump() internal virtual {}
}

interface IToken {
    function mint(uint256 amount) external;
}
