import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "Limina",
  description: "The threshold between known and unknown",
  icons: {
    icon: "/favicon.ico",
    apple: "/apple-touch-icon.png",
  },
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en" className="h-full antialiased">
      <head>
        <link
          href="https://fonts.googleapis.com/css2?family=IBM+Plex+Sans:wght@400;500;600;700&family=IBM+Plex+Mono:wght@400;500&display=swap"
          rel="stylesheet"
        />
      </head>
      <body className="min-h-full flex flex-col font-['IBM_Plex_Sans',sans-serif] bg-[#f4f4f4] text-[#161616]">
        {children}
      </body>
    </html>
  );
}
