const { Store } = require("../lib/store");

describe("Store", () => {
  test("adds", () => {
    const store = Store.create();
    expect(store.add("a", 1).size).toBe(1);
  });
});

function fixture() {
  return new Store();
}

module.exports = { fixture };
