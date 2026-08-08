import type { Metadata } from "next";
import { Geist, Geist_Mono } from "next/font/google";
import "./globals.css";

/* The hub shares the Monet type pairing with every product surface: Geist for display and body,
 * Geist Mono reserved for the data register (identifiers, status markers, hostnames). */
const geistSans = Geist({
  subsets: ["latin"],
  variable: "--font-geist-sans",
  weight: ["400", "500", "600"],
  display: "swap",
});

const geistMono = Geist_Mono({
  subsets: ["latin"],
  variable: "--font-geist-mono",
  weight: ["400", "500", "600"],
  display: "swap",
});

export const metadata: Metadata = {
  title: "Monet — metered products",
  description:
    "Monet builds small, metered products. Inver is a prepaid OpenAI-compatible inference gateway; Regretz is an anti-portfolio.",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className={`${geistSans.variable} ${geistMono.variable}`}>
      <body>{children}</body>
    </html>
  );
}
