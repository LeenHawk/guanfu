import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import tailwindcss from "@tailwindcss/vite";
import { paraglideVitePlugin } from "@inlang/paraglide-js";
import process from "node:process";
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [
    paraglideVitePlugin({
      project: "../project.inlang",
      outdir: "./src/lib/paraglide",
      strategy: ["localStorage", "baseLocale"],
      isServer: "false",
    }),
    tailwindcss(),
    sveltekit(),
  ],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || "127.0.0.1",
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching the Rust backend
      ignored: ["**/backend/**"],
    },
    proxy: {
      // realtime 走 WebSocket:代理默认不转发升级请求,必须显式开 ws。
      "/api": { target: "http://127.0.0.1:3000", ws: true },
    },
  },
}));
