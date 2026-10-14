import Foundation

/// Anything persisted by a repository.
public protocol Entity {
    var id: String { get }
    func validate() -> Bool
}

/// Something that can be rendered for humans.
protocol Describable {
    var summary: String { get }
}

extension Entity {
    // NOTE: the default validation only checks the identifier.
    func validate() -> Bool {
        return !id.isEmpty
    }
}
