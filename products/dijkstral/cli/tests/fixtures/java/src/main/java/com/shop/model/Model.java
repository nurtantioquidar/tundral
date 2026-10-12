package com.shop.model;

import java.time.Instant;
import java.util.Objects;

/**
 * Base class for stored models.
 */
public abstract class Model implements Entity<String>, Comparable<Model> {
    protected final String key;
    private Instant createdAt;

    protected Model(String key) {
        this.key = Objects.requireNonNull(key);
        this.createdAt = Instant.now();
    }

    @Override
    public String id() {
        return key;
    }

    public abstract Status status();

    @Override
    public int compareTo(Model other) {
        // NOTE: ordering is by key only.
        return key.compareTo(other.key);
    }

    /** Lifecycle states. */
    public enum Status {
        ACTIVE, SUSPENDED;

        public boolean isLive() {
            return this == ACTIVE;
        }
    }
}
