package com.shop.service

import com.shop.model.User
import kotlin.test.assertEquals
import org.junit.jupiter.api.Test

class UserServiceTest {
    private val service = UserService(InMemoryRepository())

    @Test
    fun registersUser() {
        val user: User = service.register("Ada")
        assertEquals("ada", user.name)
        service.names()
    }

    class Formatter {
        fun shout(s: String) = s
    }
}
