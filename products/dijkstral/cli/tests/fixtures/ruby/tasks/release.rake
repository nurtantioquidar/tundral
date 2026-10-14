require_relative "../lib/shop"

namespace :release do
  desc "Print the version"
  task :version do
    puts Shop::VERSION
  end

  def bump(part)
    Shop::Util.helper(part)
  end
end
