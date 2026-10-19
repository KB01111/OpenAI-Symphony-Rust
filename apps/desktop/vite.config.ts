import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), ...(process.env.VITEST ? [] : [tailwindcss()])],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test-setup.ts"],
    css: false,
    deps: {
      optimizer: {
        web: {
          enabled: true,
          include: ["@fluentui/react-components", "lucide-react"]
        }
      }
    }
  }
});
