import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// https://vitejs.dev/config/
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  // Tauri 期望固定的 dev server 端口，失败时不要弹错误覆盖层
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 不要监听 src-tauri 目录
      // 同时忽略外部工具原子写文件留下的 *.tmpdir 临时目录（监听它们会 EBUSY 崩溃）
      ignored: ["**/src-tauri/**", "**/*.tmpdir/**", "**/.*.tmp"],
    },
  },
  // 让 Tauri 暴露的环境变量在前端可用
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: "es2021",
    minify: "esbuild",
    sourcemap: false,
  },
});
