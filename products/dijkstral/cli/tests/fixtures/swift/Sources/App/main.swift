import Foundation
import Shop

let service = UserService(repository: UserRepository())

func run() throws {
    let user = try service.register(name: "ada")
    print(user.formatted().shout())
    let circle = Circle(radius: 2)
    print(circle.area())
    var box = Box<User>()
    box.put(user)
    log(Shape.unit().name)
}

try run()
