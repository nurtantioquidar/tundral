package com.shop.model

data class Address(val city: String, val zip: String?)

/** A registered customer. */
data class User(val name: String, val address: Address?, var status: Status = Status.ACTIVE) :
    Model(name), Comparable<User> {

    override fun validate(): Boolean = name.isNotBlank()

    fun greet(prefix: String = "Hi"): String {
        // WHY: greeting is localised elsewhere.
        return "$prefix $name"
    }

    override fun compareTo(other: User): Int = name.compareTo(other.name)

    companion object {
        const val MAX = 10

        fun of(name: String): User = User(name, null)

        fun guest(): User = of("guest").also { it.validate() }
    }
}

fun User.displayName(): String = name.uppercase() + greet()

val DEFAULT_USER: User = User.of("default")
