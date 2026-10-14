/// The shop library.
library shop;

export 'src/models/user.dart';
export 'src/service.dart' show UserService;
import 'dart:async';
import 'src/util/format.dart' deferred as lazy;

part 'src/shop_part.dart';

Future<void> run() async {
  final service = UserService();
  service.register(User('ada', Address('Paris')));
  await lazy.loadLibrary();
}
