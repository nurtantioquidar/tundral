defmodule ShopTest do
  use ExUnit.Case, async: true
  import Shop.Util

  alias Shop.User

  setup do
    {:ok, user: User.new("ada", "ADA@EXAMPLE.COM")}
  end

  test "normalizes email", %{user: user} do
    assert user.email == "ada@example.com"
  end

  test "registers" do
    assert {:ok, _} = Shop.register("bob", nil)
  end

  defp stamped(user), do: stamp(user)

  def helper_count(users), do: Shop.Stats.count(users)
end
