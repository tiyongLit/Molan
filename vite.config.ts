import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from '@tailwindcss/vite';
import path from "node:path"; // 确保引入了 path 模块

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [
    react(),
    tailwindcss()
  ],
  resolve: {
    alias: {
      // 将 "@" 映射到项目根目录下的 "src" 目录
      "@": path.resolve(__dirname, "./src"),
    },
  },
  // 桌面端（Tauri）前端资源本地加载，无网络下载成本；Vite 默认 500kB 警告
  // 阈值面向 Web 联网场景，按项目现实放宽到 1000kB —— 入口 chunk ≈755kB 为
  // React 系 + antd Popover 子树（Dock 静态链）+ 三语 i18n + 布局壳，
  // 页面均已路由级懒加载，重库（three/gsap 等）未进入口。
  // 入口若再显著增长（>1000kB），应重新评估拆分策略而非继续调高。
  build: {
    chunkSizeWarningLimit: 1000,
  },
  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
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
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
