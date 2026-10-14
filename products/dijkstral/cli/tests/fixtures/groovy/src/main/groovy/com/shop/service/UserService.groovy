package com.shop.service

import com.shop.model.User
import com.shop.model.Role
import com.shop.util.*
import static com.shop.util.Format.slug

class UserService implements Repository {
    private final List<User> users = []

    void register(User user) {
        users << user
        audit("register", user)
        slug(user.name)
    }

    List<String> names() {
        users.collect { u -> u.display() }
    }

    int count() {
        return users.size()
    }

    private void audit(String action, User user) {
        Logger.info(action + " " + user.getName())
    }
}

interface Repository {
    int count()
}
