module Shop
  module Util
    module_function

    # Turn any display name into a url-safe slug.
    def slugify(value)
      value.downcase.strip.gsub(/[^a-z0-9]+/, "-")
    end

    def helper(value)
      # TODO: share with the service helper.
      slugify(value)
    end
  end

  module Loggable
    def log(message)
      puts "[#{self.class.name}] #{message}"
    end
  end
end
