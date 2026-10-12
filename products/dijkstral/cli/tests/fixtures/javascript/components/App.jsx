import React, { Component } from "react";
import formatAll, { formatLabel } from "../lib/format.mjs";
import { CachedStore } from "../lib/store";

/** Renders one row. */
function Row({ label }) {
  return <li>{formatLabel(label)}</li>;
}

const List = ({ items }) => (
  <ul>
    {items.map((item) => (
      <Row key={item} label={item} />
    ))}
  </ul>
);

export default class App extends Component {
  constructor(props) {
    super(props);
    this.store = new CachedStore();
    this.onClick = this.onClick.bind(this);
  }

  onClick() {
    this.store.add("k", formatAll(["a"]));
  }

  render() {
    return <List items={["a", "b"]} onClick={this.onClick} />;
  }
}
