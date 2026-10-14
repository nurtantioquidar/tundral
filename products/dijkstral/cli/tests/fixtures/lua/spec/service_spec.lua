local Service = require("shop.service")
local shop = require("lib.shop")

describe("Service", function()
  it("adds users", function()
    local svc = Service.new()
    assert.are.equal(0, #svc:names())
  end)
end)

local function helper()
  return Service.new()
end

function make_service()
  local s = helper()
  s:add(User.new("a", 1))
  return s
end
