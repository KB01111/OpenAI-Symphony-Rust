import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { readRuntimeDiscovery } from "@symphony/api-client/node";

/** Must match `DEV_DISCOVERY_HEADER` in `packages/api-client/src/transport/discovery.ts`. */
const DEV_DISCOVERY_HEADER = "X-Symphony-Discovery";

/**
 * Development-only runtime discovery for browser sessions (`pnpm dev`, Playwright): serves the
 * local runtime's `runtime.json` endpoint + token at `/__symphony/runtime`. Not part of
 * production builds — the packaged app discovers the runtime over Tauri IPC.
 */
function symphonyDevDiscovery(): Plugin {
  return {
    name: "symphony-dev-discovery",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use("/__symphony/runtime", (req, res) => {
        // Only same-origin scripts on the dev page itself may read the token:
        // - loopback Host header (blocks DNS rebinding),
        // - the custom `X-Symphony-Discovery` header: a top-level navigation or <img>/<script>
        //   load cannot send it, and a cross-origin script cannot send it without a CORS
        //   preflight (which the Origin check below fails),
        // - browsers mark cross-site/same-site requests with Sec-Fetch-Site,
        // - an Origin header, when present, must be this dev server.
        const hostHeader = req.headers.host ?? "";
        const host = hostHeader.split(":")[0];
        const fetchSite = req.headers["sec-fetch-site"];
        const origin = req.headers.origin;
        const sameOrigin = origin === undefined || origin === `http://${hostHeader}`;
        const marked = req.headers[DEV_DISCOVERY_HEADER.toLowerCase()] === "1";
        if (
          (host !== "localhost" && host !== "127.0.0.1") ||
          !marked ||
          (fetchSite !== undefined && fetchSite !== "same-origin") ||
          !sameOrigin
        ) {
          res.statusCode = 403;
          res.end();
          return;
        }
        void readRuntimeDiscovery().then((record) => {
          res.setHeader("Content-Type", "application/json");
          res.setHeader("Cache-Control", "no-store");
          if (!record) {
            res.statusCode = 503;
            res.end(JSON.stringify({ error: "No running Symphony runtime found (start symphony-cli)." }));
            return;
          }
          res.end(JSON.stringify({ base_url: record.base_url, token: record.token }));
        });
      });
    }
  };
}

export default defineConfig({
  plugins: [react(), ...(process.env.VITEST ? [] : [tailwindcss(), symphonyDevDiscovery()])],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1"
  },
  build: {
    target: "es2022",
    chunkSizeWarningLimit: 900,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (!id.includes("node_modules")) return undefined;
          if (id.includes("@codemirror") || id.includes("/codemirror/") || id.includes("@lezer")) return "editor";
          if (id.includes("react-markdown") || id.includes("remark") || id.includes("micromark") || id.includes("mdast") || id.includes("hast") || id.includes("unified")) return "markdown";
          return undefined;
        }
      }
    }
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    // Above the 5 s that findBy/waitFor may wait (src/test/setup.ts), so a slow query fails with
    // Testing Library's message rather than a bare test timeout.
    testTimeout: 15_000,
    css: false,
    include: ["src/**/*.test.{ts,tsx}"]
  }
});
