defmodule Shop.Util do
  @moduledoc "Small helpers shared by the shop."

  def stamp(user), do: Map.put(user, :stamped_at, now())

  def normalize(%{email: nil} = user), do: user
  def normalize(%{email: email} = user), do: %{user | email: String.downcase(email)}

  defp now, do: System.system_time(:second)
end
