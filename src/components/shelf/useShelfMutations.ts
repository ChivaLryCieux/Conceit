import { useState } from "react";
import { createFolder, deleteItems, moveItem } from "../../api";
import type { Item, Toast } from "../../types";

export interface Crumb {
  id: string | null;
  title: string;
}

export interface MutationDeps {
  /** 当前所在文件夹；null = 根目录 */
  currentFolder: string | null;
  breadcrumbs: Crumb[];
  onImport: (paths: string[]) => Promise<Item[]>;
  onReload: () => Promise<void> | void;
  onToast: (text: string, tone?: Toast["tone"]) => void;
  /** 切换所在目录深度（删除当前文件夹时需要回退一层） */
  onNavigateDepth: (depth: number) => void;
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

  /** 通过系统对话框选择目录或 zip，导入到当前目录 */
  async function pickAndImport(kind: "dir" | "zip") {
    try {
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
      const paths = Array.isArray(selected) ? selected : [selected];
      setBusy(`正在导入 ${paths.length} 个项目…`);
      try {
        const imported = await onImport(paths);
        onToast(
          imported.length ? `已导入 ${imported.length} 部漫画` : "没有可导入的图片",
          imported.length ? "success" : "info"
        );
      } catch (e) {
        onToast(`导入失败：${e}`, "error");
      } finally {
        setBusy(null);
      }
    } catch (e) {
      onToast(`打开选择对话框失败：${e}`, "error");
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

  async function removeItem(item: Item) {
    const message =
      item.kind === "folder"
        ? `确定删除文件夹「${item.title}」及其中所有内容吗？`
        : `确定移除「${item.title}」吗？书库中复制的文件也会一并删除。`;
    if (!window.confirm(message)) return;
    try {
      await deleteItems([item.id]);
      // 如果删掉的正是当前所在文件夹，先退回上一层
      if (item.id === currentFolder) {
        onNavigateDepth(Math.max(0, breadcrumbs.length - 2));
      }
      await onReload();
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

  return { busy, pickAndImport, createFolderByName, removeItem, moveTo };
}
