package com.shop.model;

import java.util.List;
import java.util.ArrayList;

public class User extends Model {
    private final String name;
    private final List<Address> addresses = new ArrayList<>();

    public User(String name) {
        super(name.toLowerCase());
        this.name = name;
    }

    public User(String name, Address first) {
        this(name);
        addresses.add(first);
    }

    @Override
    public Status status() {
        return Status.ACTIVE;
    }

    public String getName() {
        return name;
    }

    public static User of(String name) {
        return new User(name);
    }

    /** A postal address, kept as a record. */
    public record Address(String street, String city) {
        public String label() {
            return street + ", " + city;
        }
    }

    /** Inner (non-static) class. */
    public class Badge {
        public String text() {
            return getName();
        }
    }
}
