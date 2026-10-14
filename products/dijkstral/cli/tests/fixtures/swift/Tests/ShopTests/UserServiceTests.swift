import XCTest
@testable import Shop

final class UserServiceTests: XCTestCase {
    var service: UserService!

    override func setUp() {
        super.setUp()
        service = UserService(repository: UserRepository())
    }

    func testRegisterStoresUser() throws {
        let user = try service.register(name: "grace")
        XCTAssertEqual(service.names(), ["Grace"])
        XCTAssertTrue(user.matches(User.make(name: "Grace")))
    }

    func testPaymentDescription() {
        let payment = Payment.failed(reason: "card")
        XCTAssertEqual(payment.describe(), "card")
    }
}
