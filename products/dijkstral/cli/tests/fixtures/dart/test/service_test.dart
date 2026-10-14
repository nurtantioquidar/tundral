import 'package:shop/shop.dart';
import 'package:test/test.dart';

void main() {
  test('registers a user', () {
    final service = UserService();
    service.register(User('ada', Address('Paris')));
    expect(service.find('ada'), isA<User>());
  });
}
