import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { readRuntimeDiscovery } from "@symphony/api-client/node";

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
        // Only same-origin requests from the dev page itself may read the token:
        // - loopback Host header (blocks DNS rebinding),
        // - browsers mark cross-site/same-site requests with Sec-Fetch-Site,
        // - an Origin header, when present, must be this dev server.
        const hostHeader = req.headers.host ?? "";
        const host = hostHeader.split(":")[0];
        const fetchSite = req.headers["sec-fetch-site"];
        const origin = req.headers.origin;
        const sameOrigin = origin === undefined || origin === `http://${hostHeader}`;
        if ((host !== "localhost" && host !== "127.0.0.1") || (fetchSite !== undefined && fetchSite !== "same-origin") || !sameOrigin) {
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
          if (id.includes("@fluentui") || id.includes("@griffel") || id.includes("tabster") || id.includes("keyborg")) return "fluent";
          if (id.includes("@openai/apps-sdk-ui") || id.includes("react-markdown") || id.includes("shiki") || id.includes("micromark") || id.includes("mdast") || id.includes("hast")) return "apps-sdk";
          return undefined;
        }
      }
    }
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    css: false,
    include: ["src/**/*.test.{ts,tsx}"],
    server: { deps: { inline: ["@openai/apps-sdk-ui"] } }
  }
});
