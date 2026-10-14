package shop.service

import shop.model.{Address, Entity, User => Customer}
import shop.model.UserOps
import shop.util._
import scala.collection.mutable
import java.time.Instant

trait Repository[T <: Entity[String]] {
  def save(item: T): Unit

  def all(): List[T]
}

/** Registers customers and keeps them in memory. */
class UserService(repo: Repository[Customer]) extends Auditable {
  private val cache: mutable.Map[String, Customer] = mutable.Map.empty
  var startedAt: Instant = Instant.now()

  @deprecated("use register(name, address)", "1.0")
  def register(name: String): Customer = {
    val user = Customer.of(slugify(name))
    repo.save(user)
    this.notifyUser(user)
    Formatter.shout(user.displayName)
    Registry.remember(user)
    user
  }

  def register(name: String, address: Address): Customer = {
    // NOTE: overload keeps the address optional.
    val user = Customer(name, Some(address))
    save(user)
    user
  }

  private def save(user: Customer): Unit = cache.update(user.id, user)

  private def notifyUser(user: Customer): Unit = {
    val task = new Runnable {
      def run(): Unit = println(user.greet())
    }
    task.run()
  }

  def names(): List[String] = repo.all().map(_.name).sorted

  override def audit(): String = s"users=${cache.size}"

  class Stats {
    def count(): Int = names().size
  }
}

object Registry {
  private val seen = mutable.ListBuffer[Customer]()

  def remember(user: Customer): Unit = {
    seen += user
    helper()
  }
}

private def helper(): Unit = println("remembered")
