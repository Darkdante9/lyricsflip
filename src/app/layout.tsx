import type { Metadata } from "next";
import { Inter } from "next/font/google";
import "./globals.css";
import { ClientProvider } from "@/components/providers/client-provider";

const inter = Inter({ subsets: ["latin"] });

export const metadata: Metadata = {
  title: {
    default: "LyricsFlip",
    template: "%s · LyricsFlip",
  },
  description:
    "LyricsFlip — flip lyrics into karaoke-ready tracks with on-chain rewards.",
  icons: {
    icon: "/favicon.ico",
  },
  openGraph: {
    title: "LyricsFlip",
    description:
      "LyricsFlip — flip lyrics into karaoke-ready tracks with on-chain rewards.",
    url: "https://lyricsflip.app",
    siteName: "LyricsFlip",
    images: [
      {
        url: "/og-image.png",
        width: 1200,
        height: 630,
        alt: "LyricsFlip",
      },
    ],
    locale: "en_US",
    type: "website",
  },
  twitter: {
    card: "summary_large_image",
    title: "LyricsFlip",
    description:
      "LyricsFlip — flip lyrics into karaoke-ready tracks with on-chain rewards.",
    images: ["/twitter-image.png"],
  },
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en">
      <body className={inter.className}>
        <ClientProvider>{children}</ClientProvider>
      </body>
    </html>
  );
}
