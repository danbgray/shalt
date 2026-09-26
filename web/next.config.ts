import type { NextConfig } from "next";

const engine = process.env.SHALT_ENGINE_URL?.replace(/\/$/, "") || "";

const nextConfig: NextConfig = {
  async rewrites() {
    const local = [{ source: "/desk", destination: "/ui.html" }];
    if (!engine) return local;
    const proxy = [
      "org",
      "events",
      "jobs/:path*",
      "project/:path*",
      "models",
      "keys",
      "compose",
      "browse",
      "import",
    ].map((p) => ({
      source: `/api/${p}`,
      destination: `${engine}/api/${p}`,
    }));
    return {
      beforeFiles: local,
      afterFiles: proxy,
      fallback: [],
    };
  },
};

export default nextConfig;
