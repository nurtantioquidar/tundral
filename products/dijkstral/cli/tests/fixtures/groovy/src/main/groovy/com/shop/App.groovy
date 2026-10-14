package com.shop

import com.shop.service.UserService
import com.shop.model.User

class App {
    static void main(String[] args) {
        def service = new UserService()
        service.register(new User("ada"))
        service.names().each { println it }
        report(service)
    }

    static void report(UserService service) {
        Banner.show(service.count())
    }
}
