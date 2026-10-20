import { useState } from "react";
import { Channel } from "@tauri-apps/api/core";
import {
  clearImportCache,
  createFolder,
  deleteItems,
  isAndroid,
  moveItem,
  pickDirectory,
  pickZipFiles,
  type ImportProgress,
} from "../../api";
import type { Item, Toast } from "../../types";

export interface Crumb {
  id: string | null;
  title: string;
}

/** 进度条视图数据：ratio 为 null 表示总量未知（不确定态动画） */
export interface ProgressView {
  ratio: number | null;
  label: string;
}

export interface MutationDeps {
  /** 当前所在文件夹；null = 根目录 */
  currentFolder: string | null;
  breadcrumbs: Crumb[];
  onImport: (paths: string[], channel: Channel<ImportProgress>) => Promise<Item[]>;
  onReload: () => Promise<void> | void;
  onToast: (text: string, tone?: Toast["tone"]) => void;
  /** 切换所在目录深度（删除当前文件夹时需要回退一层） */
  onNavigateDepth: (depth: number) => void;
}

/** 把后端回传的进度换算成进度条要显示的内容 */
function toProgressView(p: ImportProgress): ProgressView {
  const ratio =
    p.totalBytes > 0
      ? Math.min(1, Math.max(0, p.doneBytes / p.totalBytes))
      : null;
  const pct = ratio === null ? "" : ` ${Math.round(ratio * 100)}%`;
  const counter = p.total > 1 ? `（${p.index}/${p.total}）` : "";
  const label =
    p.phase === "thumb"
      ? `正在生成封面…${counter}`
      : `正在导入「${p.title}」${counter}${pct}`;
  return { ratio, label };
}

/** 书架的数据操作：导入、新建文件夹、删除、移动。
 *  对话框插件保持动态引入，避免打进主包。 */
export function useShelfMutations({
  currentFolder,
  breadcrumbs,
  onImport,
  onReload,
  onToast,
  onNavigateDepth,
}: MutationDeps) {
  const [busy, setBusy] = useState<string | null>(null);
  const [progress, setProgress] = useState<ProgressView | null>(null);

  /** 通过系统对话框选择目录或 zip，导入到当前目录 */
  async function pickAndImport(kind: "dir" | "zip") {
    const android = await isAndroid();
    // 一次导入共用一个 Channel：安卓上 SAF 复制与 Rust 书库复制两段都往里报
    const channel = new Channel<ImportProgress>((msg) =>
      setProgress(toProgressView(msg))
    );
    try {
      let paths: string[];
      if (android) {
        // Android：插件对话框不支持选目录，文件选择器返回的也是 content:// URI，
        // std::fs 用不了——走内置原生桥，直接拿到缓存里的真实路径
        // （见 src-tauri/src/mobile_import.rs 与 src-tauri/android/ConceitPlugin.kt）
        if (kind === "dir") {
          // 选完目录后原生侧要先统计总大小做空间预估，可能有几秒等待，
          // 先把遮罩挂上；不清 busy 直接进入「正在导入」，避免闪烁
          setBusy("正在读取所选目录…");
          const dir = await pickDirectory(channel);
          if (!dir) {
            setBusy(null);
            return; // 用户取消
          }
          paths = [dir];
        } else {
          paths = await pickZipFiles(channel);
          if (paths.length === 0) return; // 用户取消
        }
      } else {
        const { open } = await import("@tauri-apps/plugin-dialog");
        const selected =
          kind === "dir"
            ? await open({
                multiple: true,
                directory: true,
                title: "选择要导入的漫画文件夹（可多选）",
              })
            : await open({
                multiple: true,
                title: "选择 zip 漫画压缩包",
                filters: [{ name: "漫画压缩包", extensions: ["zip"] }],
              });
        if (!selected) return;
        paths = Array.isArray(selected) ? selected : [selected];
      }
      setBusy(`正在导入 ${paths.length} 个项目…`);
      // 安卓 SAF 阶段可能只报了「读取中」，进入书库复制前清掉旧进度，
      // 让进度条从这一部的 0 重新开始（两个阶段的文件名不同，不会误读）
      setProgress(null);
      try {
        const imported = await onImport(paths, channel);
        onToast(
          imported.length ? `已导入 ${imported.length} 部漫画` : "没有可导入的图片",
          imported.length ? "success" : "info"
        );
      } catch (e) {
        onToast(`导入失败：${e}`, "error");
      } finally {
        setBusy(null);
        setProgress(null);
        if (android) {
          // 书库已有自己的副本，清掉 Kotlin 侧复制进缓存的临时文件
          await clearImportCache().catch(() => {});
        }
      }
    } catch (e) {
      // 覆盖对话框打开失败与原生侧读取失败（空间不足、SAF 提供器报错等）
      onToast(`读取所选内容失败：${e}`, "error");
      setBusy(null);
      setProgress(null);
    }
  }

  /** 新建文件夹；成功返回 true（失败时保留弹窗供重试） */
  async function createFolderByName(name: string): Promise<boolean> {
    try {
      await createFolder(name, currentFolder);
      await onReload();
      onToast(`已创建文件夹「${name}」`, "success");
      return true;
    } catch (e) {
      onToast(`创建文件夹失败：${e}`, "error");
      return false;
    }
  }

  /** 删除条目：文件夹递归删除其中内容；漫画连同磁盘目录与封面缩略图一并删除。
   *  删掉的正是当前所在文件夹时先退回上一层。二次确认由调用方负责
   *  （ConfirmDialog——window.confirm 在安卓 WebView 上会被直接取消）。 */
  async function deleteItem(item: Item) {
    try {
      await deleteItems([item.id]);
      // 如果删掉的正是当前所在文件夹，先退回上一层
      if (item.id === currentFolder) {
        onNavigateDepth(Math.max(0, breadcrumbs.length - 2));
      }
      await onReload();
      onToast(`已删除「${item.title}」`, "success");
    } catch (e) {
      onToast(`删除失败：${e}`, "error");
    }
  }

  async function moveTo(item: Item, target: string | null) {
    try {
      await moveItem(item.id, target);
      await onReload();
    } catch (e) {
      onToast(`移动失败：${e}`, "error");
    }
  }

  return {
    busy,
    progress,
    pickAndImport,
    createFolderByName,
    deleteItem,
    moveTo,
  };
}
