import Foundation

class Shape {
    var name: String

    init(name: String) {
        self.name = name
    }

    func area() -> Double {
        return 0
    }

    class func unit() -> Shape {
        return Shape(name: "unit")
    }
}

final class Circle: Shape, Describable {
    let radius: Double

    init(radius: Double) {
        self.radius = radius
        super.init(name: "circle")
    }

    override func area() -> Double {
        return Double.pi * radius * radius
    }

    var summary: String {
        return "\(name) r=\(radius)"
    }
}

/// A generic container.
struct Box<T> {
    var items: [T] = []

    mutating func put(_ item: T) {
        items.append(item)
    }

    func first() -> T? {
        return items.first
    }
}

struct Pair<Key: Hashable, Value> where Value: Equatable {
    let key: Key
    let value: Value
}
