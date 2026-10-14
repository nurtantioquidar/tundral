"""
Anything that can be stored in the registry.
"""
abstract type Entity end

abstract type Person <: Entity end

struct User <: Person
    name::String
    email::Union{Nothing, String}
    created::DateTime
end

mutable struct Admin <: Person
    user::User
    level::Int
end

struct Box{T} <: Entity
    value::T
    label::Label
end

struct Label
    text::String
end

primitive type Flags 8 end

name(u::User) = u.name
name(a::Admin) = name(a.user)
