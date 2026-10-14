-- Shop package entry point.
local util = require("shop.util")
local User = require "shop.models.user"
local Service = require('shop.service')

local M = {}
M.VERSION = "1.0"

--- Build a service with a default user.
function M.new(name)
  local svc = Service.new()
  svc:add(name)
  return svc
end

function M.version()
  return util.format_price(M.VERSION)
end

return M
