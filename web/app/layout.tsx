import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "Shalt",
  description: "English becomes tests, then code. The spec still wins.",
  metadataBase: new URL("https://shalt.dev"),
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
