package com.shop.model

import groovy.transform.CompileStatic
import groovy.transform.ToString
import com.shop.util.Format

@CompileStatic
@ToString(includeNames = true)
class User extends Base implements Named, Comparable<User>, Auditable {
    String name;
    Address address;

    User(String name) {
        this.name = name;
    }

    User(String name, Address address) {
        this(name);
        this.address = address;
    }

    String getName() {
        return Format.title(name);
    }

    String display() {
        return describe() + " " + getName();
    }

    int compareTo(User other) {
        return name.compareTo(other.name);
    }

    static class Address {
        String city;

        String label() {
            return Format.title(city);
        }
    }
}

enum Role {
    ADMIN, MEMBER, GUEST;

    boolean canWrite() {
        return this == ADMIN;
    }
}
