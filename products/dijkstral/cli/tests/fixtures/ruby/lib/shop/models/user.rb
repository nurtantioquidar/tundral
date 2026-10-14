require_relative "base"

module Shop
  module Models
    # A registered customer.
    class User < Base
      include Comparable
      extend Forwardable
      prepend Shop::Loggable

      attr_accessor :name, :role
      attr_writer :status

      def initialize(name, role: :member)
        super()
        @name = Util.slugify(name)
        @role = role
      end

      def self.parse(raw)
        new(raw.strip)
      end

      def save
        # WHY: users are validated twice because the parent check is cheap.
        ok = super
        ok && validate
      end

      def display
        [name, role].map { |part| part.to_s.capitalize }.join(" ")
      end

      def <=>(other)
        name <=> other.name
      end

      class Admin < User
        def promote(user)
          user.role = :admin
          log("promoted #{user.name}")
        end
      end
    end
  end
end

class Shop::Models::Guest < Shop::Models::User
  def display
    "guest"
  end
end
