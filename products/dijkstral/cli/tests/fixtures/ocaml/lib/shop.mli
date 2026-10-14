type user = { name : string; email : string option }

val register : string -> string option -> user
