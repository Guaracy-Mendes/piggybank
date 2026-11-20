import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
const apiTarget = process.env.BANK_API_PROXY_TARGET ?? "http://127.0.0.1:4180";
const apiUrl = new URL(apiTarget);
if (
  apiUrl.protocol !== "http:" ||
  apiUrl.hostname !== "127.0.0.1" ||
  apiUrl.pathname !== "/" ||
  apiUrl.username ||
  apiUrl.password ||
  apiUrl.search ||
  apiUrl.hash
)
  throw new Error("The local portal requires a loopback API target.");
export default defineConfig({
  plugins: [
    react(),
    {
      name: "customer-trust-boundary",
      configureServer(server) {
        server.middlewares.use((request, response, next) => {
          const parsed = new URL(request.url ?? "/", "http://channel.invalid");
          const path = parsed.pathname;
          if (
            parsed.origin !== "http://channel.invalid" ||
            ((path.startsWith("/api") || path.startsWith("/auth")) &&
              path.includes("%"))
          ) {
            response.statusCode = 403;
            response.end("Use the canonical path for this channel.");
            return;
          }
          if (
            path.startsWith("/api/v1/staff") ||
            path.startsWith("/api/v1/providers") ||
            /^\/auth\/(workforce|service)/.test(path)
          ) {
            response.statusCode = 403;
            response.end("This channel requires a different trust context.");
            return;
          }
          next();
        });
      },
    },
  ],
  server: {
    host: "127.0.0.1",
    port: Number(process.env.BANK_CUSTOMER_WEB_PORT ?? 4300),
    strictPort: true,
    proxy: {
      "/api": apiTarget,
      "/auth/customer": apiTarget,
    },
  },
  build: { sourcemap: false },
});
