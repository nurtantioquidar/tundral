package shop.util

/** Lowercase, dash-separated form of `s`. */
def slugify(s: String): String = s.trim.toLowerCase.replace(' ', '-')

object Formatter {
  def shout(s: String): String = s.toUpperCase

  def whisper(s: String): String = slugify(s)
}

extension (s: String) def words: List[String] = s.split(' ').toList
