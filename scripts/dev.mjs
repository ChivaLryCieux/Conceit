/**
 * dev 启动包装：vite + ESBUILD_BINARY_PATH 修复。
 *
 * 背景：某些环境下 pnpm store 中的 esbuild.exe 被设置了
 * “Mandatory Label\Low Mandatory Level (NW)”（低完整性级别、禁止向上写）。
 * 该标签会随硬链接进入 node_modules，导致 esbuild 服务进程无法创建
 * 目录 / 写文件，vite 的依赖预打包随之失败，dev 模式下页面白屏
 * （生产构建不受影响：产物由 Rollup 写入，esbuild 只在内存中转换）。
 *
 * 做法：复制一份“干净”的 esbuild 二进制到 node_modules/.esbuild-local/，
 * 通过 ESBUILD_BINARY_PATH 让 esbuild 使用它。正常机器上同样无害。
 */
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

if (process.platform === "win32") {
  const pnpmDir = path.join(root, "node_modules", ".pnpm");
  let binary = null;
  if (fs.existsSync(pnpmDir)) {
    for (const entry of fs.readdirSync(pnpmDir)) {
      if (!entry.startsWith("@esbuild+win32-x64@")) continue;
      const candidate = path.join(
        pnpmDir,
        entry,
        "node_modules",
        "@esbuild",
        "win32-x64",
        "esbuild.exe"
      );
      if (fs.existsSync(candidate)) {
        binary = candidate;
        break;
      }
    }
  }
  if (binary) {
    const localDir = path.join(root, "node_modules", ".esbuild-local");
    const localBin = path.join(localDir, "esbuild.exe");
    try {
      fs.mkdirSync(localDir, { recursive: true });
      const src = fs.statSync(binary);
      let stale = true;
      if (fs.existsSync(localBin)) {
        const dst = fs.statSync(localBin);
        stale = dst.size !== src.size || dst.mtimeMs < src.mtimeMs;
      }
      if (stale) fs.copyFileSync(binary, localBin);
      process.env.ESBUILD_BINARY_PATH = localBin;
      console.log(
        `[dev] 使用副本 esbuild：${localBin}（规避 Low 完整性标签导致的写入限制）`
      );
    } catch {
      // 复制失败时沿用默认二进制
    }
  }
}

const viteBin = path.join(root, "node_modules", "vite", "bin", "vite.js");
const child = spawn(process.execPath, [viteBin, ...process.argv.slice(2)], {
  cwd: root,
  stdio: "inherit",
});

child.on("exit", (code, signal) => {
  if (signal) process.kill(process.pid, signal);
  else process.exit(code ?? 1);
});
