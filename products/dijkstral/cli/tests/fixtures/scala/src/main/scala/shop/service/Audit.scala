package shop.service

import shop.model.Entity

class Audited(action: String) extends scala.annotation.StaticAnnotation

trait Auditable {
  def audit(): String
}

trait Logging { self: Auditable =>
  def log(msg: String): Unit = println(s"${audit()}: $msg")
}

class InMemoryRepository[T <: Entity[String]] extends Repository[T] with Logging with Auditable {
  private val items = scala.collection.mutable.ArrayBuffer[T]()

  override def save(item: T): Unit = {
    items += item
    log("saved")
  }

  override def all(): List[T] = items.toList

  override def audit(): String = s"items=${items.size}"
}
