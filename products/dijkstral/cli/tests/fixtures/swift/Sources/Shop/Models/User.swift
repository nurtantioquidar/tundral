import Foundation

/// The role a user plays in the shop.
public enum Role: String, Codable {
    case admin
    case customer, guest

    var isPrivileged: Bool {
        return self == .admin
    }
}

/// A payment state with associated values.
enum Payment {
    case pending
    case paid(amount: Int, at: Date)
    case failed(reason: String)

    func describe() -> String {
        switch self {
        case .pending: return "pending"
        case .paid(let amount, _): return "paid \(amount)"
        case .failed(let reason): return reason
        }
    }
}

/// A registered customer.
public struct User: Entity, Equatable {
    public let id: String
    var name: String
    var role: Role
    var address: Address?
    static let anonymous = User(id: "0", name: "anon")

    /// Builds a user with the customer role.
    init(id: String, name: String) {
        self.id = id
        self.name = name
        self.role = .customer
    }

    init(id: String, name: String, role: Role) {
        self.id = id
        self.name = name
        self.role = role
    }

    var displayName: String {
        get { name.capitalized }
    }

    // WHY: names are normalised before comparison to avoid duplicate accounts.
    func matches(_ other: User) -> Bool {
        return normalized(name) == normalized(other.name)
    }

    private func normalized(_ value: String) -> String {
        return value.lowercased()
    }

    static func make(name: String) -> User {
        return User(id: UUID().uuidString, name: name)
    }

    mutating func rename(to newName: String) {
        name = newName
    }
}

struct Address: Codable {
    var street: String
    var city: String
}
