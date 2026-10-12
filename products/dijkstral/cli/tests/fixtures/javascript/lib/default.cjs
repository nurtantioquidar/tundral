const { Store } = require("./store");
const store = require("./store.js");
const lodash = require("lodash");

function helper() {
  return lodash.identity(Store.create());
}

module.exports = function makeDefault() {
  return store.helper(helper());
};
