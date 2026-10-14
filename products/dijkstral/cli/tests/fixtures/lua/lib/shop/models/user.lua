local Base = require("shop.models.base")
local util = require("shop.util")

local User = setmetatable({}, { __index = Base })
User.__index = User

-- WHY: users are created through User.new so the metatable is always set.
function User.new(name, price)
  local self = Base.new("user")
  setmetatable(self, User)
  self.name = name
  self.price = util.round(price)
  return self
end

function User:display()
  return self.name .. " " .. self:describe() .. " " .. M.format_price(self.price)
end

function User:id()
  return #self.name
end

function User.Admin.promote(u)
  u.admin = true
  return u
end

return User
