open Shop

module D = Make (struct let prefix = "user: " end)

let () =
  let u = register "ada" (Some "ADA@EXAMPLE.COM") in
  print_endline (D.describe u)
