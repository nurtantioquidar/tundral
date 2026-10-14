package shop.model

case class Address(city: String, zip: Option[String])

/** A registered customer. */
case class User(name: String, address: Option[Address], status: Status = Active)
    extends Model(name)
    with Ordered[User] {

  override def validate(): Boolean = name.nonEmpty

  def greet(prefix: String = "Hi"): String = {
    // WHY: greeting is localised elsewhere.
    s"$prefix $name"
  }

  def compare(that: User): Int = name.compare(that.name)
}

object User {
  val Max = 10

  def of(name: String): User = User(name, None)

  def guest(): User = of("guest")
}

implicit class UserOps(u: User) {
  def displayName: String = u.name.toUpperCase + u.greet()
}

val DefaultUser: User = User.of("default")
