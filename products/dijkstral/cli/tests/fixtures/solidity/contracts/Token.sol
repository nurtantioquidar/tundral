// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "./Base.sol";

contract Token is Base, IToken {
    uint256 public total;
    event Minted(address to, uint256 amount);

    function mint(uint256 amount) external override {
        total += amount;
        bump();
        emit Minted(msg.sender, amount);
    }
}
