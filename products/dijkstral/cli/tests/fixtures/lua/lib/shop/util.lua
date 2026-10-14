-- Utility helpers.
-- NOTE: keep these pure; no I/O here.
local M = {}

local function clamp(x, lo, hi)
  if x < lo then return lo end
  return math.min(x, hi)
end

--- Format a price with two decimals.
function M.format_price(price)
  return string.format("%.2f", clamp(price, 0, 1e9))
end

function M.round(x)
  return clamp(math.floor(x + 0.5), 0, 100)
end

M.trim = function(s)
  return (s:gsub("^%s+", ""))
end

function slugify(s)
  return string.lower(M.trim(s))
end

return M
