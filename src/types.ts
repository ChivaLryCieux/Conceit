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

export const ROOT_KEY = "root";

/** 轻提示 */
export interface Toast {
  id: number;
  text: string;
  tone: "info" | "error" | "success";
}
