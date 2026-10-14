defmodule Shop.Format do
  alias Shop.User

  def line(%User{name: name, email: email}), do: "#{name} <#{email}>"
  def line(_), do: "?"

  defprotocol Printable do
    def print(value)
  end

  defimpl Printable, for: User do
    def print(user), do: Shop.Format.line(user)
  end
end

defimpl String.Chars, for: Shop.User do
  def to_string(user), do: Shop.Format.line(user)
end
