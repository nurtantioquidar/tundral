local Base = {}
Base.__index = Base

function Base.new(kind)
  local self = setmetatable({}, Base)
  self.kind = kind
  return self
end

function Base:describe()
  return self.kind .. ":" .. tostring(self:id())
end

function Base:id()
  return 0
end

return Base
