package com.shop

import com.shop.model.User
import com.shop.service.UserService
import spock.lang.Specification

class UserServiceSpec extends Specification {
    UserService service

    def setup() {
        service = new UserService()
    }

    def "registers a user"() {
        when:
        service.register(new User("ada"))

        then:
        service.count() == 1
    }

    def 'lists names'() {
        given:
        service.register(new User("bob"))

        expect:
        service.names() == ["Bob"]
    }
}
