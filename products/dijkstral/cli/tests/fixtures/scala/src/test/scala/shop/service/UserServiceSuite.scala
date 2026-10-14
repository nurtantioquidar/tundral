package shop.service

import shop.model.User
import munit.FunSuite

class UserServiceSuite extends FunSuite {
  private val service = new UserService(new InMemoryRepository[User]())

  test("registers a user") {
    val user: User = service.register("Ada")
    assertEquals(user.name, "ada")
    service.names()
  }

  object Formatter {
    def shout(s: String): String = s
  }
}
