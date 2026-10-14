require "json"
require "set"
require_relative "shop/util"
require_relative "shop/models/base"
require_relative "shop/models/user"
require_relative "shop/service"

# Top-level namespace for the shop fixture.
module Shop
  VERSION = "1.0.0"

  def self.boot(name)
    service = Service::UserService.new(Models::Repo.new)
    service.register(name)
  end
end
