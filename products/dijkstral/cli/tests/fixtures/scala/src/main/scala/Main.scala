import shop.service.{InMemoryRepository, UserService}

object Main {
  def main(args: Array[String]): Unit = {
    val service = new UserService(new InMemoryRepository())
    service.register("Grace")
    scala()
  }
}

def scala(): Unit = ()
