import Foundation
import Combine

protocol Repository {
    associatedtype Item: Entity
    func save(_ item: Item) throws
    func find(id: String) -> Item?
}

final class UserRepository: Repository {
    typealias Item = User
    private var store: [String: User] = [:]

    func save(_ item: User) throws {
        guard item.validate() else { throw ShopError.invalid(item.id) }
        store[item.id] = item
    }

    func find(id: String) -> User? {
        return store[id]
    }

    func all() -> [User] {
        return Array(store.values)
    }
}

enum ShopError: Error {
    case invalid(String)
    case missing
}

actor Cache {
    private var hits: [String: User] = [:]

    func lookup(_ id: String) -> User? {
        return hits[id]
    }

    func remember(_ user: User) {
        hits[user.id] = user
    }
}

/// Coordinates user registration.
class UserService {
    let repository: UserRepository
    private let cache = Cache()
    var onRegister: ((User) -> Void)?

    init(repository: UserRepository) {
        self.repository = repository
    }

    func register(name: String) throws -> User {
        let user = User.make(name: name)
        try repository.save(user)
        notify(user)
        return user
    }

    func register(name: String, role: Role) throws -> User {
        let user = User(id: UUID().uuidString, name: name, role: role)
        try repository.save(user)
        return user
    }

    func lookup(id: String) async -> User? {
        if let hit = await cache.lookup(id) {
            return hit
        }
        guard let user = repository.find(id: id) else { return nil }
        await cache.remember(user)
        return user
    }

    private func notify(_ user: User) {
        onRegister?(user)
        log(user.formatted())
    }

    func names() -> [String] {
        return repository.all().map { $0.displayName }
    }
}

func log(_ message: String) {
    print(message)
}
