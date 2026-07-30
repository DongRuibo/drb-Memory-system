import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  metadataBase: new URL(
    process.env.NEXT_PUBLIC_SITE_URL ?? "http://localhost:3000",
  ),
  title: "LazyTT Memory · 会记忆的 AI",
  description: "用一条自然对话，直观看懂 AI 如何了解用户、项目和过去经验。",
  openGraph: {
    title: "LazyTT Memory · 会记忆的 AI",
    description: "直观看懂 AI 如何了解用户、项目和过去经验。",
    images: [{ url: "/og.png", width: 1728, height: 911 }],
  },
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="zh-CN">
      <body>{children}</body>
    </html>
  );
}
