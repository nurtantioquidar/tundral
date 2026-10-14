import 'dart:collection';
import 'models/user.dart';
import 'util/format.dart';

abstract interface class Repository<T> {
  void store(T item);
  List<T> everything();
}

class UserService implements Repository<User> {
  final Map<String, User> _users = HashMap<String, User>();
  late Account owner;

  @override
  void save(User user) => _users[user.name] = user;

  @override
  List<User> all() => _users.values.toList();

  void register(User user) {
    save(user);
    log(title(user.name));
  }

  User? find(String name) => _users[name];

  bool isUser(Object value) => value is User && isA<User>(value);
}

bool isA<T>(Object v) => v is T;
