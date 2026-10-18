// 与 Rust 端 library.rs 对应的数据模型

export type ItemKind = "manga" | "folder";

export interface MangaMeta {
  /** 相对书库根目录 */
  dir: string;
  /** 封面相对路径 */
  cover?: string;
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
}

export const ROOT_KEY = "root";
