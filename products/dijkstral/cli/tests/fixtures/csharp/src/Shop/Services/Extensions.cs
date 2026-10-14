using System.Linq;

namespace Shop.Services
{
    public static class StringExtensions
    {
        /// <summary>Upper-cases and adds an exclamation mark.</summary>
        public static string Shout(this string value) => value.ToUpperInvariant() + "!";

        public static string Slug(this string value)
        {
            return new string(value.Where(char.IsLetterOrDigit).ToArray()).ToLowerInvariant();
        }
    }

    internal static class Formatter
    {
        public static string Format(string text) => text.Shout();
    }
}
