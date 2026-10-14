/// Root of every model.
abstract class Entity {
  String get id;

  // NOTE: equality is by id.
  bool sameAs(Entity other) => id == other.id;
}

mixin Printable on Entity {
  void printMe() => print(describe());

  String describe() => 'Entity($id)';
}

class Comparable2 {}
