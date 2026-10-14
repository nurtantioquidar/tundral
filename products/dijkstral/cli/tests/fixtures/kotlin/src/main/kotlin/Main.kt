import com.shop.service.UserService
import com.shop.service.InMemoryRepository

fun main() {
    val service = UserService(InMemoryRepository())
    service.register("Grace")
    kt()
}

fun kt() {}
