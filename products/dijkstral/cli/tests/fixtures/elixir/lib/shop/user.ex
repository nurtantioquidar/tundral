defmodule Shop.User do
  @enforce_keys [:name]
  defstruct [:name, :email, role: :member]

  @type t :: %__MODULE__{name: String.t(), email: String.t() | nil}

  def new(name, email \\ nil)

  def new(name, email) when is_binary(name) do
    %__MODULE__{name: name, email: email} |> Shop.Util.normalize()
  end

  def new(_name, _email), do: {:error, :invalid_name}

  def admin?(%__MODULE__{role: :admin}), do: true
  def admin?(_), do: false

  def promote!(user), do: %{user | role: :admin}

  defmacro is_member(user) do
    quote do: unquote(user).role == :member
  end

  defguard is_named(user) when is_map_key(user, :name)

  defdelegate format(user), to: Shop.Format, as: :line
end
