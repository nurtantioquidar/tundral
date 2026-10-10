import type { Metadata } from "next";
import { JetBrains_Mono } from "next/font/google";
import "./globals.css";

/* One family, two weights (DESIGN.md § Type): JetBrains Mono 700 for headings, 400 for text. */
const jetbrains = JetBrains_Mono({
  subsets: ["latin"],
  variable: "--font-jetbrains",
  weight: ["400", "700"],
  display: "swap",
});

export const metadata: Metadata = {
  title: "Monet",
  description:
    "Monet is a small studio named for the painter. It builds instruments for working with AI: Butterfly, Cocoon, Nimble, Inver, and Cicada. Next, the art made with them.",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className={jetbrains.variable}>
      <body>{children}</body>
    </html>
  );
}
