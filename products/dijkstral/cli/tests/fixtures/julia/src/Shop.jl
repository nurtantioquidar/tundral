module Shop

using Dates
import Base: show

export User, Admin, register, describe

include("models.jl")
include("service.jl")

# NOTE: the registry is process-global on purpose.
const REGISTRY = Dict{String, Any}()

module Internal
    using ..Shop: Entity

    function count_entities(xs)
        length(filter(x -> x isa Entity, xs))
    end
end

end # module
