import "@openuidev/react-ui/defaults.css";
import "@openuidev/react-ui/styles/index.css";
import "./globals.css";
import type { Metadata } from "next";
import type { ReactNode } from "react";

export const metadata: Metadata = {
  title: "lang-core vs openui-rs",
  description: "Upstream OpenUI's JavaScript parser and openui-rs's WebAssembly core under the same React renderer.",
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" data-openui-theme="light">
      <body>{children}</body>
    </html>
  );
}
