import { useState } from "react";
import Card from "./Card";
import ContextMenu, { type MenuAction } from "./ContextMenu";
import {
  childrenOf,
  countManga,
  createFolder,
  deleteItems,
  moveItem,
  renameItem,
} from "../api";
import type { Item, LibrarySnapshot } from "../types";
import type { Toast } from "../App";

interface Props {
  snapshot: LibrarySnapshot;
  currentFolder: string | null;
  breadcrumbs: { id: string | null; title: string }[];
  onEnterFolder: (id: string) => void;
  onNavigateDepth: (depth: number) => void;
  onOpenManga: (item: Item) => void;
  onReload: () => Promise<void> | void;
  onToast: (text: string, tone?: Toast["tone"]) => void;
  onImport: (paths: string[]) => Promise<Item[]>;
}

export default function Shelf({
  snapshot,
  currentFolder,
  breadcrumbs,
  onEnterFolder,
  onNavigateDepth,
  onOpenManga,
  onReload,
  onToast,
  onImport,
}: Props) {
  const items = childrenOf(snapshot.index, currentFolder);
  const [busy, setBusy] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; item: Item } | null>(
    null
  );

  function activate(item: Item) {
    if (item.kind === "folder") onEnterFolder(item.id);
    else onOpenManga(item);
  }

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

  async function handleNewFolder() {
    const name = window.prompt("新文件夹名称", "新建文件夹");
    if (name == null) return;
    const trimmed = name.trim();
    if (!trimmed) return;
    try {
      await createFolder(trimmed, currentFolder);
      await onReload();
    } catch (e) {
      onToast(`创建文件夹失败：${e}`, "error");
    }
  }

  async function handleRename(item: Item) {
    const name = window.prompt("重命名", item.title);
    if (name == null) return;
    const trimmed = name.trim();
    if (!trimmed || trimmed === item.title) return;
    try {
      await renameItem(item.id, trimmed);
      await onReload();
    } catch (e) {
      onToast(`重命名失败：${e}`, "error");
    }
  }

  async function handleDelete(item: Item) {
    const message =
      item.kind === "folder"
        ? `确定删除文件夹「${item.title}」及其中所有内容吗？`
        : `确定移除「${item.title}」吗？书库中复制的文件也会一并删除。`;
    if (!window.confirm(message)) return;
    try {
      await deleteItems([item.id]);
      await onReload();
    } catch (e) {
      onToast(`删除失败：${e}`, "error");
    }
  }

  async function handleMoveToRoot(item: Item) {
    if (!item.parent) return;
    try {
      await moveItem(item.id, null);
      await onReload();
    } catch (e) {
      onToast(`移动失败：${e}`, "error");
    }
  }

  function menuActions(item: Item): MenuAction[] {
    const actions: MenuAction[] = [
      {
        label: item.kind === "folder" ? "打开文件夹" : "开始阅读",
        onSelect: () => activate(item),
      },
      { label: "重命名", onSelect: () => void handleRename(item) },
    ];
    if (item.parent) {
      actions.push({
        label: "移动到根目录",
        onSelect: () => void handleMoveToRoot(item),
      });
    }
    actions.push({
      label: "删除",
      danger: true,
      separatorBefore: true,
      onSelect: () => void handleDelete(item),
    });
    return actions;
  }

  return (
    <div className="shelf">
      <header className="shelf-header">
        <div className="shelf-brand">
          <span className="brand-mark">📚</span>
          <span className="brand-name">Conceit</span>
        </div>
        <nav className="crumbs">
          {breadcrumbs.map((c, i) => (
            <span key={c.id ?? "root"} className="crumb-wrap">
              {i > 0 && <span className="crumb-sep">/</span>}
              <button
                type="button"
                className={`crumb${
                  i === breadcrumbs.length - 1 ? " crumb-current" : ""
                }`}
                onClick={() => onNavigateDepth(i)}
              >
                {c.title}
              </button>
            </span>
          ))}
        </nav>
        <div className="shelf-actions">
          <button
            type="button"
            className="btn"
            onClick={() => void pickAndImport("dir")}
          >
            ＋ 导入文件夹
          </button>
          <button
            type="button"
            className="btn"
            onClick={() => void pickAndImport("zip")}
          >
            ＋ 导入 zip
          </button>
          <button
            type="button"
            className="btn btn-primary"
            onClick={() => void handleNewFolder()}
          >
            新建文件夹
          </button>
        </div>
      </header>

      {items.length === 0 ? (
        <div className="shelf-empty">
          <p className="shelf-empty-title">这个书架还空着</p>
          <p>点击「导入文件夹」选择本机漫画目录，或把文件夹 / zip 直接拖到窗口里</p>
          <button
            type="button"
            className="btn btn-primary"
            onClick={() => void pickAndImport("dir")}
          >
            导入漫画
          </button>
        </div>
      ) : (
        <div className="shelf-grid">
          {items.map((item) => (
            <Card
              key={item.id}
              item={item}
              root={snapshot.root}
              mangaCount={
                item.kind === "folder"
                  ? countManga(snapshot.index, item.id)
                  : undefined
              }
              indicator={{ mode: null }}
              dragging={false}
              onActivate={() => activate(item)}
              onContextMenu={(e) =>
                setMenu({ x: e.clientX, y: e.clientY, item })
              }
              onDragStart={() => {}}
              onDragEnd={() => {}}
              onDrop={() => {}}
            />
          ))}
        </div>
      )}

      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          actions={menuActions(menu.item)}
          onClose={() => setMenu(null)}
        />
      )}

      {busy && (
        <div className="busy-mask">
          <div className="spinner" />
          <div>{busy}</div>
        </div>
      )}
    </div>
  );
}
