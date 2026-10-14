package com.shop.service

import com.shop.model.Address
import com.shop.model.Entity
import com.shop.model.User as Customer
import com.shop.model.displayName
import com.shop.util.*
import kotlinx.coroutines.flow.Flow
import org.slf4j.LoggerFactory

interface Repository<T : Entity<String>> {
    fun save(item: T)

    fun all(): List<T>
}

/** Registers customers and keeps them in memory. */
class UserService(private val repo: Repository<Customer>) : Auditable {
    private val log = LoggerFactory.getLogger("users")
    private val cache: MutableMap<String, Customer> = mutableMapOf()
    lateinit var updates: Flow<Customer>

    @Audited("register")
    fun register(name: String): Customer {
        val user = Customer.of(slugify(name))
        repo.save(user)
        this.notify(user)
        Formatter.shout(user.displayName())
        Registry.remember(user)
        return user
    }

    fun register(name: String, address: Address): Customer {
        // NOTE: overload keeps the address optional.
        val user = Customer(name, address)
        save(user)
        return user
    }

    private fun save(user: Customer) {
        cache[user.id] = user
    }

    private fun notify(user: Customer) {
        val task = object : Runnable {
            override fun run() {
                log.info(user.greet())
            }
        }
        task.run()
    }

    fun names(): List<String> = repo.all().map { it.name }.sorted()

    override fun audit(): String = "users=${cache.size}"

    inner class Stats {
        fun count(): Int = names().size
    }
}

object Registry {
    private val seen = mutableListOf<Customer>()

    fun remember(user: Customer) {
        seen.add(user)
        helper()
    }
}

private fun helper() {
    println("remembered")
}
