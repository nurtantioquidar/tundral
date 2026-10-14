-- Luau module (typed Lua superset).
local Server = {}
Server.__index = Server

type Config = {
	port: number,
}

function Server.new(config: Config): Server
	local self = setmetatable({}, Server)
	self.port = config.port
	return self
end

function Server:start(): ()
	print(string.format("listening on :%d", self.port))
end

local function boot()
	local s = Server.new({ port = 8080 })
	s:start()
end

boot()

return Server
