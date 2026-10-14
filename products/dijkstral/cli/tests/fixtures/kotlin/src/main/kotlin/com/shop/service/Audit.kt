package com.shop.service

annotation class Audited(val action: String)

interface Auditable {
    fun audit(): String
}

class InMemoryRepository<T : com.shop.model.Entity<String>> : Repository<T> {
    private val items = ArrayList<T>()

    override fun save(item: T) {
        items.add(item)
    }

    override fun all(): List<T> = items.toList()
}
