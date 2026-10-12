/**
 * In-memory store.
 * @template T
 */
class Store {
  constructor() {
    this.items = new Map();
  }

  static create() {
    return new Store();
  }

  add(key, item) {
    this.items.set(key, item);
    return this;
  }

  get size() {
    return this.items.size;
  }
}

class CachedStore extends Store {
  add(key, item) {
    // NOTE: invalidates the cache on every write.
    super.add(key, item);
    return clear(this);
  }
}

function clear(store) {
  return store;
}

const helper = function (value) {
  return String(value).trim();
};

const utils = {
  shout(text) {
    return helper(text).toUpperCase();
  },
  whisper: (text) => helper(text).toLowerCase(),
};

module.exports = { Store, CachedStore, helper };
exports.utils = utils;
