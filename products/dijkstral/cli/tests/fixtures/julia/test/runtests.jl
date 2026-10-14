using Test
using Shop

struct Fixture
    admin::Admin
    label::Label
end

function make_user()
    Shop.register("ada", "ada@example.com")
end

@testset "shop" begin
    u = make_user()
    @test Shop.describe(u) == "ada <ada@example.com>"
end
