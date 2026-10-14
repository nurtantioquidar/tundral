using Printf

# WHY: dispatch picks the right describe for every entity.
function describe(u::User)
    @sprintf("%s <%s>", name(u), u.email)
end

function describe(a::Admin)
    string(describe(a.user), " (level ", a.level, ")")
end

describe(e::Entity) = "entity"

function register(name::String, email=nothing)
    user = User(name, email, now())
    REGISTRY[name] = user
    validate(user) || error("invalid")
    return user
end

validate(u::User) = !isempty(u.name)

function Base.show(io::IO, u::User)
    print(io, describe(u))
end

(l::Label)(suffix) = l.text * suffix

macro registered(name)
    :(haskey(REGISTRY, $(esc(name))))
end

function promote(u::User; level::Int=1)
    a = Admin(u, level)
    map(x -> validate(x.user), [a])
    return a
end
