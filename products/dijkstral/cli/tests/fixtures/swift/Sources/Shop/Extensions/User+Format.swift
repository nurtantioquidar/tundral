import Foundation

extension User: CustomStringConvertible {
    public var description: String {
        return formatted()
    }

    func formatted() -> String {
        return "\(name) <\(role.rawValue)>"
    }
}

extension User: Describable {
    var summary: String { formatted() }
}

extension Array where Element == User {
    func sortedByName() -> [User] {
        return sorted { $0.name < $1.name }
    }
}

extension String {
    func shout() -> String {
        return uppercased()
    }
}
