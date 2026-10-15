import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // Static export: the Rust binary embeds web/out via rust-embed and serves
  // the panel itself, so no Node runtime is needed on the server.
  output: "export",
  // The panel is hosted under /admin/ by the gateway binary.
  basePath: "/admin",
  // Emit /accounts/index.html style paths so the embedded static file server
  // can resolve client routes directly.
  trailingSlash: true,
  images: {
    unoptimized: true,
  },
  eslint: {
    // CI builds should not fail on lint; keep lint as a separate step.
    ignoreDuringBuilds: true,
  },
};

export default nextConfig;
