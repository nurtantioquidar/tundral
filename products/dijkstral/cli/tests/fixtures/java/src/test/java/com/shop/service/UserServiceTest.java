package com.shop.service;

import static org.junit.jupiter.api.Assertions.assertEquals;

import com.shop.model.User;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

class UserServiceTest {
    private UserService service;

    @BeforeEach
    void setUp() {
        service = new UserService();
    }

    @Test
    void registersUser() {
        User user = service.register("Alice");
        assertEquals("alice", user.id());
    }

    /** Duplicate name: Formatter also exists in main. */
    static class Formatter {
        String format(String s) {
            return s;
        }
    }
}
