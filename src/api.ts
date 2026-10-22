// 对 Rust 命令的薄封装

import { invoke, type Channel } from "@tauri-apps/api/core";
import type {
  HealthReport,
  Item,
  LibraryIndex,
  LibrarySnapshot,
} from "./types";
import { ROOT_KEY } from "./types";

/** 导入进度：Rust 与安卓原生桥（SAF 复制）经同一 Channel 回传，字段一致 */
export interface ImportProgress {
  /** 阶段：copy = 复制 / 解压，thumb = 生成封面缩略图 */
  phase: "copy" | "thumb";
  /** 当前是第几部（从 1 开始） */
  index: number;
  /** 一共几部 */
  total: number;
  /** 当前漫画标题 */
  title: string;
  /** 当前这部已处理的字节数 */
  doneBytes: number;
  /** 当前这部预计总字节数；0 = 未知（进度条走不确定态） */
  totalBytes: number;
}

export async function fetchSnapshot(): Promise<LibrarySnapshot> {
  return invoke<LibrarySnapshot>("library_snapshot");
}

/// 从本机路径导入漫画（目录或 zip），复制进书库；targetFolder 为 null 表示根目录。
/// channel 用于把复制 / 解压的字节进度实时推给前端（进度条）
export async function importSources(
  paths: string[],
  targetFolder: string | null,
  channel: Channel<ImportProgress>
): Promise<Item[]> {
  return invoke<Item[]>("import_sources", { paths, targetFolder, channel });
}

export async function createFolder(
  name: string,
  parent: string | null
): Promise<Item> {
  return invoke<Item>("create_folder", { name, parent });
}

/** 新建一部空漫画（还没有图片），之后在设置里「＋ 添加图片」逐张补充；
 *  parent 为 null 表示根目录 */
export async function createEmptyManga(
  name: string,
  parent: string | null
): Promise<Item> {
  return invoke<Item>("create_empty_manga", { name, parent });
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

/* ---------- 书库体检 ---------- */

/** 体检：把磁盘实况与索引对比，返回全部差异（含每个漫画目录里实际有什么） */
export async function libraryHealth(): Promise<HealthReport> {
  return invoke<HealthReport>("library_health");
}

export interface HealthCleanupTarget {
  id: string;
  /** 是否连同磁盘目录一起删除 */
  deleteFiles: boolean;
}

/** 按体检结果清理：移除书架条目与 / 或删除磁盘目录，返回处理的条目数 */
export async function libraryHealthClean(
  targets: HealthCleanupTarget[]
): Promise<number> {
  return invoke<number>("library_health_clean", { targets });
}

/** 按磁盘实况重新扫描某部漫画的页数 / 封面 / 缩略图，返回更新后的条目 */
export async function rescanMangaPages(id: string): Promise<Item> {
  return invoke<Item>("rescan_manga_pages", { id });
}

/** 把磁盘上已有的漫画目录收进书架（不复制文件），返回新条目 */
export async function adoptMangaDir(
  id: string,
  title: string,
  parent: string | null
): Promise<Item> {
  return invoke<Item>("adopt_manga_dir", { id, title, parent });
}

export async function fetchPages(id: string): Promise<string[]> {
  return invoke<string[]>("manga_pages", { id });
}

/* ---------- Android 原生桥（仅 Android 有意义，桌面端为报错存根） ----------
 * Android 上 tauri-plugin-dialog 不支持目录选择，文件选择器返回的也是
 * content:// URI 而非文件系统路径；这些命令由内置 Kotlin 插件
 * （src-tauri/android/ConceitPlugin.kt）返回缓存里的真实路径。 */

/** 打开系统目录选择器，返回缓存中的真实目录路径；取消返回 null。
 *  channel 由原生侧在 SAF 复制过程中回报进度（可选，不传则没有进度） */
export async function pickDirectory(
  channel: Channel<ImportProgress>
): Promise<string | null> {
  return invoke<string | null>("pick_directory", { channel });
}

/** 打开系统文件选择器（zip，可多选），返回缓存中的真实文件路径；
 *  channel 由原生侧在复制过程中回报进度 */
export async function pickZipFiles(
  channel: Channel<ImportProgress>
): Promise<string[]> {
  return invoke<string[]>("pick_zip_files", { channel });
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

/** 系统栏高度（px，安卓原生桥提供）：top = 状态栏，bottom = 导航栏。
 *  供顶栏 / 底栏避让使用；桌面端存根报错，由调用方回退 env() */
export async function systemBarInsets(): Promise<{ top: number; bottom: number }> {
  return invoke("system_bar_insets");
}

/** 打开系统图片选择器（安卓可多选），返回缓存中的真实路径；取消返回空列表 */
export async function pickImages(): Promise<string[]> {
  return invoke<string[]>("pick_images");
}

/* ---------- 漫画图片管理（设置页） ---------- */

/** 给漫画追加图片：复制进书库并排在末尾，返回更新后的条目 */
export async function addMangaPages(id: string, paths: string[]): Promise<Item> {
  return invoke<Item>("add_manga_pages", { id, paths });
}

/** 删除漫画中的若干页（文件一并删除），返回更新后的条目；
 *  封面页被删时会自动顺延，需要从返回值同步封面状态 */
export async function deleteMangaPages(id: string, pages: string[]): Promise<Item> {
  return invoke<Item>("delete_manga_pages", { id, pages });
}

/** 自定义漫画图片顺序，返回更新后的条目 */
export async function reorderMangaPages(id: string, pages: string[]): Promise<Item> {
  return invoke<Item>("reorder_manga_pages", { id, pages });
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
