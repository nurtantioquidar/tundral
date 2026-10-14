/// Formatting helpers.
String title(String s) => s.isEmpty ? s : s[0].toUpperCase() + s.substring(1);

void log(String message) {
  final stamp = DateTime.now();
  print('$stamp $message');
}

const defaultWidth = 80;
