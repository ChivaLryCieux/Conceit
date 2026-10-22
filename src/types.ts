// 与 Rust 端 library.rs 对应的数据模型

export type ItemKind = "manga" | "folder";

export interface MangaMeta {
  /** 相对书库根目录 */
  dir: string;
  /** 封面相对路径（漫画目录内的某一页） */
  cover?: string;
  /** 封面缩略图路径（相对书库根目录，如 thumbs/m1.jpg）；
   *  缺失时前端回退显示原图封面 */
  thumb?: string;
  /** 全部图片相对路径（自然排序） */
  pages: string[];
  /** 阅读进度 0-1 */
  progress: number;
}

export interface Item {
  id: string;
  kind: ItemKind;
  title: string;
  /** 所属文件夹 id；undefined 表示根目录 */
  parent?: string;
  /** 是否展示封面图；false = 纯色封面（默认）。
   *  注意：Tauri 只把命令「参数」在 camelCase/snake_case 间自动转换，
   *  响应体键名与 Rust serde 输出一致，因此这里用蛇形。 */
  show_cover: boolean;
  manga?: MangaMeta;
  added_at: number;
}

export interface LibraryIndex {
  version: number;
  items: Item[];
  /** 容器键（"root" 或文件夹 id）-> 有序子条目 id */
  order: Record<string, string[]>;
  next_seq: number;
}

export interface LibrarySnapshot {
  root: string;
  index: LibraryIndex;
  /** 文件夹 id -> 其中（含子文件夹）的漫画数量，由 Rust 预计算 */
  manga_counts: Record<string, number>;
}

/* ---------- 书库体检 ---------- */

/** 漫画目录在磁盘上的实况 */
export interface DirStat {
  /** 目录名（即漫画 id，如 m3） */
  id: string;
  /** 索引里的标题；索引中没有该目录时为 null（孤儿目录） */
  title: string | null;
  /** 磁盘上文件总数（跳过隐藏文件与 __MACOSX） */
  files: number;
  /** 其中图片数（书架实际能读到的页数） */
  images: number;
  /** 目录总字节 */
  bytes: number;
  /** 前若干个文件名，让用户直接看到目录里装了什么 */
  sample: string[];
}

/** 索引里有条目、磁盘上却找不到对应目录 */
export interface MissingEntry {
  id: string;
  title: string;
  /** 索引里记录的页数 */
  pages: number;
}

export interface DuplicateMember {
  id: string;
  /** 索引里记录的页数 */
  pages: number;
  /** 磁盘上实际的图片数 */
  images: number;
  /** 目录字节数 */
  bytes: number;
  /** 加入书库的时间（Unix 秒） */
  added_at: number;
  /** 阅读进度 0-1 */
  progress: number;
  /** 该成员的目录是否已丢失 */
  dir_missing: boolean;
}

/** 疑似重复的一组漫画 */
export interface DuplicateGroup {
  title: string;
  /** pages_identical = 页面集合完全相同，基本可确定是同一部漫画导了两次；
   *  same_title_same_count = 同名同页数但文件名不同 */
  reason: "pages_identical" | "same_title_same_count";
  members: DuplicateMember[];
}

/** 索引页数与磁盘图片数不一致 */
export interface PageDrift {
  id: string;
  title: string;
  /** 索引里记录的页数 */
  indexed: number;
  /** 磁盘上实际的图片数 */
  on_disk: number;
}

/** 书库体检报告：磁盘实况与索引的全部差异 */
export interface HealthReport {
  /** 索引中的漫画条目数 */
  indexed: number;
  /** 磁盘 manga/ 下的目录数 */
  on_disk: number;
  missing_dirs: MissingEntry[];
  orphan_dirs: DirStat[];
  duplicates: DuplicateGroup[];
  page_drift: PageDrift[];
  /** 磁盘上每个漫画目录的实况（含孤儿目录） */
  disk_stats: DirStat[];
}

export const ROOT_KEY = "root";

/** 轻提示 */
export interface Toast {
  id: number;
  text: string;
  tone: "info" | "error" | "success";
}
