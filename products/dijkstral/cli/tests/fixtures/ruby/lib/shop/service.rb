require "logger"
require_relative "models/user"

module Shop::Service
  class UserService
    include Shop::Loggable

    def initialize(repo)
      @repo = repo
      @logger = Logger.new($stdout)
    end

    def register(name)
      user = Models::User.new(name)
      user.save
      @repo.add(Util.slugify(name), user)
      self.audit(user)
      log("registered #{name}")
      user
    end

    def promote(admin, user)
      admin.promote(user)
      Shop::Models::User.parse(user.name)
    end

    def export
      @repo.map { |u| helper(u.name) }.sort_by(&:length)
    end

    private

    def audit(user)
      helper(user.name)
    end

    def helper(value)
      value.to_s
    end
  end

  def self.build
    UserService.new(Models::Repo.new)
  end
end
