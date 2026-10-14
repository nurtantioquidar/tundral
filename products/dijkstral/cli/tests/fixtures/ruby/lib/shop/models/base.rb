module Shop
  module Models
=begin
Base model shared by every persisted entity in the shop.
=end
    class Base
      attr_reader :id

      def initialize(id = nil)
        @id = id
      end

      def save
        # NOTE: persistence is faked in this fixture.
        validate
      end

      def validate
        true
      end

      def self.table
        name.downcase
      end
    end

    class Repo
      include Enumerable

      def initialize
        @items = {}
      end

      def add(key, item)
        @items[key] = item
      end

      def each(&block)
        @items.each_value(&block)
      end
    end
  end
end
