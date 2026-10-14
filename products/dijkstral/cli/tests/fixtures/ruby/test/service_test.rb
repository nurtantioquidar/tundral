require "minitest/autorun"
require_relative "../lib/shop"

class ServiceTest < Minitest::Test
  def setup
    @service = Shop::Service.build
  end

  def test_register
    user = @service.register("Ada")
    assert_equal "ada", user.name
  end

  def test_parse
    user = Shop::Models::User.parse(" bob ")
    assert_equal "Bob Member", user.display
  end
end
