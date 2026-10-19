// 对 Rust 命令的薄封装

import { invoke } from "@tauri-apps/api/core";
import type { Item, LibraryIndex, LibrarySnapshot } from "./types";
import { ROOT_KEY } from "./types";

export async function fetchSnapshot(): Promise<LibrarySnapshot> {
  return invoke<LibrarySnapshot>("library_snapshot");
}

/// 从本机路径导入漫画（目录或 zip），复制进书库；targetFolder 为 null 表示根目录
export async function importSources(
  paths: string[],
  targetFolder: string | null
): Promise<Item[]> {
  return invoke<Item[]>("import_sources", { paths, targetFolder });
}

export async function createFolder(
  name: string,
  parent: string | null
): Promise<Item> {
  return invoke<Item>("create_folder", { name, parent });
}

export async function renameItem(id: string, title: string): Promise<void> {
  return invoke("rename_item", { id, title });
}

export interface ItemSettings {
  title?: string;
  showCover?: boolean;
  /** 选择漫画某一页作为封面（该页的相对路径） */
  cover?: string;
}

/** 更新条目设置：名称 / 封面开关 / 封面页 */
export async function updateItemSettings(
  id: string,
  settings: ItemSettings
): Promise<void> {
  return invoke("update_item_settings", {
    id,
    title: settings.title ?? null,
    showCover: settings.showCover ?? null,
    cover: settings.cover ?? null,
  });
}

export async function deleteItems(ids: string[]): Promise<void> {
  return invoke("delete_items", { ids });
}

export async function moveItem(
  id: string,
  newParent: string | null
): Promise<void> {
  return invoke("move_item", { id, newParent: newParent });
}

export async function reorder(
  parent: string | null,
  orderedIds: string[]
): Promise<void> {
  return invoke("reorder", { parent, orderedIds });
}

export async function saveProgress(id: string, progress: number): Promise<void> {
  return invoke("save_progress", { id, progress });
}

export async function fetchPages(id: string): Promise<string[]> {
  return invoke<string[]>("manga_pages", { id });
}

/* ---------- Android 原生桥（仅 Android 有意义，桌面端为报错存根） ----------
 * Android 上 tauri-plugin-dialog 不支持目录选择，文件选择器返回的也是
 * content:// URI 而非文件系统路径；这些命令由内置 Kotlin 插件
 * （src-tauri/android/ConceitPlugin.kt）返回缓存里的真实路径。 */

/** 打开系统目录选择器，返回缓存中的真实目录路径；取消返回 null */
export async function pickDirectory(): Promise<string | null> {
  return invoke<string | null>("pick_directory");
}

/** 打开系统文件选择器（zip，可多选），返回缓存中的真实文件路径 */
export async function pickZipFiles(): Promise<string[]> {
  return invoke<string[]>("pick_zip_files");
}

/** 清理导入桥复制进缓存的临时文件 */
export async function clearImportCache(): Promise<void> {
  return invoke("clear_import_cache");
}

/** 是否运行在 Android 上（决定导入走原生桥还是插件对话框） */
export async function isAndroid(): Promise<boolean> {
  try {
    return (await invoke<string>("app_platform")) === "android";
  } catch {
    return false;
  }
}

/** 退出应用：安卓系统返回在书架根目录时调用（桌面端存根报错，正常不会触达） */
export async function exitApp(): Promise<void> {
  return invoke("exit_app");
}

/** 某个容器（root 或文件夹）下按排序排列的条目 */
export function childrenOf(
  index: LibraryIndex,
  parent: string | null
): Item[] {
  const key = parent ?? ROOT_KEY;
  const ordered = index.order[key] ?? [];
  const byId = new Map(index.items.map((it) => [it.id, it]));
  const result: Item[] = [];
  const seen = new Set<string>();
  for (const id of ordered) {
    const item = byId.get(id);
    if (item && item.parent === (parent ?? undefined) && !seen.has(id)) {
      result.push(item);
      seen.add(id);
    }
  }
  // 兜底：补齐未排序的新条目
  for (const item of index.items) {
    if (
      (item.parent ?? undefined) === (parent ?? undefined) &&
      !seen.has(item.id)
    ) {
      result.push(item);
      seen.add(item.id);
    }
  }
  return result;
}
