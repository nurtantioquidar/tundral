defmodule Shop do
  @moduledoc """
  Entry points for the shop.
  """

  alias Shop.{Repo, User}
  alias Shop.Format, as: Fmt
  require Logger

  # NOTE: registration always goes through the repository.
  def register(name, email) do
    name
    |> User.new(email)
    |> Repo.insert()
    |> log_result()
  end

  def describe(user), do: Fmt.line(user)

  def admins(users) do
    Enum.filter(users, &User.admin?/1)
  end

  defp log_result({:ok, user} = result) do
    Logger.info("registered #{user.name}")
    result
  end

  defp log_result(error), do: error

  defmodule Stats do
    @moduledoc false
    # WHY: nested so it stays private to the shop.
    def count(users), do: length(users)
    def summary(users), do: "#{count(users)} users"
  end

  def summary(users), do: Stats.summary(users)
end
