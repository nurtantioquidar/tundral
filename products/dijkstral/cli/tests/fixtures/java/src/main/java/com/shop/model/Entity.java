package com.shop.model;

/** Anything with an identity. */
public interface Entity<K> {
    K id();

    default String describe() {
        return "entity " + id();
    }
}
