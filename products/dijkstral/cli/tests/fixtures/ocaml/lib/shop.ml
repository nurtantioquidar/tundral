open Printf

(* NOTE: users are immutable records. *)
type user = { name : string; email : string option }

module Registry = struct
  let users : user list ref = ref []
  let add u = users := u :: !users
end

module Make (S : sig val prefix : string end) = struct
  let describe u = sprintf "%s%s" S.prefix u.name
end

let normalize email = Option.map String.lowercase_ascii email

let register name email =
  let u = { name; email = normalize email } in
  Registry.add u;
  u
