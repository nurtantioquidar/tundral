defmodule Shop.Repo do
  use GenServer
  import Shop.Util, only: [stamp: 1]

  def start_link(opts), do: GenServer.start_link(__MODULE__, opts, name: __MODULE__)

  @impl true
  def init(state), do: {:ok, state}

  def insert(%Shop.User{} = user) do
    user = stamp(user)
    GenServer.call(__MODULE__, {:insert, user})
  end

  def insert(other), do: {:error, other}

  @impl true
  def handle_call({:insert, user}, _from, state) do
    {:reply, {:ok, user}, [user | state]}
  end

  def all, do: GenServer.call(__MODULE__, :all)
end
