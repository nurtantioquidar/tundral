import 'package:meta/meta.dart';
import 'base.dart';
import 'package:shop/src/util/format.dart' as fmt;

@immutable
class User extends Entity with Printable implements Comparable2 {
  final String name;
  final Address home;
  static const int maxLen = 10;

  User(this.name, this.home);
  factory User.guest() => User('guest', Address('none'));

  @override
  String get id => name;

  String display() {
    return fmt.title(name) + home.city;
  }

  int compareTo(User other) => name.compareTo(other.name);
}

class Address {
  final String city;
  Address(this.city);
}

enum Role { admin, member }

extension UserLabels on User {
  String get label => display();
}

typedef Account = User;
typedef Validator = bool Function(User user);
