let () =
  let u = Shop.register "bob" None in
  assert (u.Shop.name = "bob")
