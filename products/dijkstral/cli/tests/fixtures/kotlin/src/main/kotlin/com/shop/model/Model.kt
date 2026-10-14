package com.shop.model

import java.time.Instant

/** Anything with a stable identifier. */
interface Entity<K> {
    val id: K

    fun describe(): String = "entity $id"
}

/**
 * Base class for persisted models.
 * NOTE: timestamps are in UTC.
 */
abstract class Model(override val id: String) : Entity<String> {
    val createdAt: Instant = Instant.now()

    abstract fun validate(): Boolean

    fun touch(): Model {
        validate()
        return this
    }
}

enum class Status {
    ACTIVE,
    INACTIVE;

    fun label(): String = name.lowercase()
}

sealed class Result {
    data class Ok(val value: String) : Result()

    object Fail : Result()
}

typealias UserId = String
