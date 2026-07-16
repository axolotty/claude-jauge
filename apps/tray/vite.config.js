import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],
  // Tauri affiche ses propres erreurs dans ce terminal : ne les effacons pas.
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
});
