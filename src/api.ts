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

export async function fetchCover(id: string): Promise<string | null> {
  return invoke<string | null>("manga_cover", { id });
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

/** 递归统计一个文件夹内的漫画数量 */
export function countManga(index: LibraryIndex, folderId: string): number {
  let count = 0;
  for (const item of index.items) {
    if (item.kind !== "manga") continue;
    let cur: string | undefined = item.parent;
    while (cur) {
      if (cur === folderId) {
        count += 1;
        break;
      }
      cur = index.items.find((i) => i.id === cur)?.parent;
    }
  }
  return count;
}
