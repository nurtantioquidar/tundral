alias Shop.Repo

for name <- ~w(ada bob) do
  Shop.register(name, nil)
end

Repo.all()
