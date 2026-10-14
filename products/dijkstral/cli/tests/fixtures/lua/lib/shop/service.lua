local User = require("shop.models.user")
local json = require("dkjson")

local Service = {}
Service.__index = Service

function Service.new()
  return setmetatable({ users = {} }, Service)
end

function Service:add(user)
  table.insert(self.users, user)
  self:log("added " .. user.name)
end

function Service:log(msg)
  print(msg)
end

function Service:names()
  local out = {}
  for _, u in ipairs(self.users) do
    out[#out + 1] = u:display()
  end
  table.sort(out, function(a, b)
    return slugify(a) < slugify(b)
  end)
  return out
end

function Service:export()
  local function encode(v)
    return json.encode(v)
  end
  return encode(self:names())
end

function Service:promote_all()
  for _, u in ipairs(self.users) do
    User.Admin.promote(u)
    u:display()
  end
end

return Service
