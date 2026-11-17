import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig(({ mode }) => ({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // Tests read the design reference sheet for the token drift guard.
    fs: mode === "test" ? { allow: [".", "../design"] } : undefined,
  },
  resolve: mode === "test" ? { conditions: ["browser"] } : undefined,
  test: { environment: "jsdom", include: ["src/**/*.test.ts"], css: { include: [/app\.css/] } },
}));
