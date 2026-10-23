import type { Metadata } from "next";
import { Geist, Geist_Mono } from "next/font/google";
import "./globals.css";

/* Coral runs a single-family display/body pairing (modern-minimal discipline) with the mono as the
 * outlier register for code, masked secrets, IDs, and tabular numerics. */
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
  title: "Inver — prepaid inference gateway",
  description:
    "OpenAI-compatible model access on prepaid balance. Every request reserves, settles, and posts to an immutable ledger in integer micro-dollars.",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className={`${geistSans.variable} ${geistMono.variable}`}>
      <body>{children}</body>
    </html>
  );
}
