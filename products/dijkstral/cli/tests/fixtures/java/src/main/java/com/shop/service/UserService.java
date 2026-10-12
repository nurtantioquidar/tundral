package com.shop.service;

import com.shop.model.*;
import com.shop.model.User.Address;
import java.util.*;
import java.util.function.Function;
import static java.util.Objects.requireNonNull;
import static com.shop.service.Formatter.slug;

/** Registers users and keeps them in memory. */
public class UserService implements Repository<User> {
    private final Map<String, User> users = new HashMap<>();
    private final Formatter formatter = new Formatter();

    @Audited("register")
    public User register(String name) {
        User user = User.of(requireNonNull(name));
        save(user);
        this.notify(user);
        formatter.format(user.getName());
        Formatter.shout(slug(name));
        return user;
    }

    public User register(String name, Address address) {
        // WHY: overload keeps the address optional.
        User user = new User(name, address);
        save(user);
        return user;
    }

    @Override
    public void save(User user) {
        users.put(user.id(), user);
    }

    private void notify(User user) {
        Runnable r = new Runnable() {
            @Override
            public void run() {
                System.out.println(user.getName());
            }
        };
        r.run();
    }

    public List<String> names() {
        Function<User, String> f = User::getName;
        List<String> out = new ArrayList<>();
        users.values().forEach(u -> out.add(f.apply(u)));
        users.values().stream().map(User::getName).forEach(out::add);
        Collections.sort(out);
        return out;
    }

    public static class Stats {
        public int count(UserService service) {
            return service.names().size();
        }
    }
}

interface Repository<T extends Entity<String>> {
    void save(T item);
}

class Formatter {
    String format(String s) {
        return s.trim();
    }

    static String shout(String s) {
        return s.toUpperCase();
    }

    static String slug(String s) {
        return s.replace(' ', '-');
    }
}
