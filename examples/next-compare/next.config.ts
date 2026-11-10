import path from "node:path";
import type { NextConfig } from "next";

// Every import of @openuidev/lang-core, react-lang's included, gets the shim;
// the shim reaches the real package by file path, which no alias matches.
const shim = "./lib/lang-core-shim.ts";

const nextConfig: NextConfig = {
  turbopack: {
    root: path.resolve(import.meta.dirname),
    resolveAlias: { "@openuidev/lang-core": shim },
  },
  // Cross-origin isolation: without it browsers coarsen performance.now() to
  // 0.1 ms (Chrome) or worse, too coarse for parser calls that take ~0.05 ms.
  async headers() {
    return [
      {
        source: "/:path*",
        headers: [
          { key: "Cross-Origin-Opener-Policy", value: "same-origin" },
          { key: "Cross-Origin-Embedder-Policy", value: "require-corp" },
        ],
      },
    ];
  },
  // Only used with `next build --webpack`; Turbopack is the default.
  webpack(config) {
    config.resolve.alias["@openuidev/lang-core$"] = path.resolve(import.meta.dirname, shim);
    return config;
  },
};

export default nextConfig;
