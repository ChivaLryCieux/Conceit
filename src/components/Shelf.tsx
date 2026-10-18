import { useState } from "react";
import Card, { type DropMode } from "./Card";
import ContextMenu, { type MenuAction } from "./ContextMenu";
import {
  childrenOf,
  countManga,
  createFolder,
  deleteItems,
  moveItem,
  reorder,
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

interface OverState {
  id: string | null;
  mode: DropMode;
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
  /** 拖拽中的条目 id */
  const [dragId, setDragId] = useState<string | null>(null);
  /** 拖放落点指示 */
  const [over, setOver] = useState<OverState>({ id: null, mode: "before" });
  /** 面包屑上的落点（索引） */
  const [crumbOver, setCrumbOver] = useState<number | null>(null);

  function activate(item: Item) {
    if (item.kind === "folder") onEnterFolder(item.id);
    else onOpenManga(item);
  }

  /* ---------- 导入 ---------- */

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

  /* ---------- 书架操作 ---------- */

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
      // 如果删掉的正是当前所在文件夹，先退回上一层
      if (item.id === currentFolder) {
        onNavigateDepth(Math.max(0, breadcrumbs.length - 2));
      }
      await onReload();
    } catch (e) {
      onToast(`删除失败：${e}`, "error");
    }
  }

  async function handleMoveTo(item: Item, target: string | null) {
    try {
      await moveItem(item.id, target);
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
        onSelect: () => void handleMoveTo(item, null),
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

  /* ---------- 拖拽排序 ---------- */

  function onCardDragStart(e: React.DragEvent, item: Item) {
    setDragId(item.id);
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", item.id);
  }

  function onCardDragEnd() {
    setDragId(null);
    setOver({ id: null, mode: "before" });
  }

  /** 根据鼠标位置计算落点：文件夹=进入，漫画=前/后，空白=末尾，自己=原位 */
  function computeOver(e: React.DragEvent): OverState {
    const el = (e.target as HTMLElement).closest<HTMLElement>("[data-card-id]");
    if (!el) return { id: null, mode: "before" };
    const id = el.dataset.cardId ?? null;
    if (!id) return { id: null, mode: "before" };
    if (id === dragId) return { id, mode: "before" };
    const item = snapshot.index.items.find((i) => i.id === id);
    if (item?.kind === "folder") return { id, mode: "into" };
    const rect = el.getBoundingClientRect();
    const before = e.clientX < rect.left + rect.width / 2;
    return { id, mode: before ? "before" : "after" };
  }

  function onGridDragOver(e: React.DragEvent) {
    if (!dragId) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
    setCrumbOver(null);
    const next = computeOver(e);
    if (next.id === dragId) return; // 悬停在自己身上时保持当前指示
    setOver(next);
  }

  async function onGridDrop(e: React.DragEvent) {
    if (!dragId) return;
    e.preventDefault();
    const movingId = dragId;
    const target = computeOver(e);
    setDragId(null);
    setOver({ id: null, mode: "before" });
    if (target.id === movingId) return;

    try {
      if (target.mode === "into" && target.id) {
        await moveItem(movingId, target.id);
        await onReload();
        return;
      }
      const ids = items.map((i) => i.id).filter((x) => x !== movingId);
      if (target.id) {
        const idx = ids.indexOf(target.id);
        const at = target.mode === "before" ? idx : idx + 1;
        ids.splice(idx < 0 ? ids.length : at, 0, movingId);
      } else {
        ids.push(movingId);
      }
      await reorder(currentFolder, ids);
      await onReload();
    } catch (err) {
      onToast(`排序失败：${err}`, "error");
    }
  }

  function onCrumbDragOver(e: React.DragEvent, index: number) {
    if (!dragId) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
    setOver({ id: null, mode: "before" });
    setCrumbOver(index);
  }

  async function onCrumbDrop(e: React.DragEvent, folderId: string | null) {
    if (!dragId) return;
    e.preventDefault();
    const movingId = dragId;
    setDragId(null);
    setCrumbOver(null);
    if (movingId === folderId) return;
    await handleMoveTo(
      snapshot.index.items.find((i) => i.id === movingId)!,
      folderId
    );
  }

  /* ---------- 渲染 ---------- */

  return (
    <div className="shelf">
      <header className="shelf-header">
        <div className="shelf-brand">
          <span className="brand-mark">📚</span>
          <span className="brand-name">幻想阅读器</span>
          <span className="brand-sub">Conceit</span>
        </div>
        <nav className="crumbs">
          {breadcrumbs.map((c, i) => (
            <span key={c.id ?? "root"} className="crumb-wrap">
              {i > 0 && <span className="crumb-sep">/</span>}
              <button
                type="button"
                className={`crumb${
                  i === breadcrumbs.length - 1 ? " crumb-current" : ""
                }${crumbOver === i ? " crumb-droppable" : ""}`}
                onClick={() => onNavigateDepth(i)}
                onDragOver={(e) => onCrumbDragOver(e, i)}
                onDragLeave={() => setCrumbOver(null)}
                onDrop={(e) => void onCrumbDrop(e, c.id)}
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
          <p>
            点击「导入文件夹」选择本机漫画目录，或把文件夹 / zip
            直接拖到窗口里
          </p>
          <button
            type="button"
            className="btn btn-primary"
            onClick={() => void pickAndImport("dir")}
          >
            导入漫画
          </button>
        </div>
      ) : (
        <div
          className="shelf-grid"
          onDragOver={onGridDragOver}
          onDrop={(e) => void onGridDrop(e)}
        >
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
              indicator={
                over.id === item.id && over.id !== dragId
                  ? { mode: over.mode }
                  : { mode: null }
              }
              dragging={dragId === item.id}
              onActivate={() => activate(item)}
              onContextMenu={(e) =>
                setMenu({ x: e.clientX, y: e.clientY, item })
              }
              onDragStart={(e) => onCardDragStart(e, item)}
              onDragEnd={onCardDragEnd}
              onDrop={(e) => {
                e.stopPropagation();
                onGridDrop(e);
              }}
            />
          ))}
          {dragId && over.id === null && <div className="drop-endline" />}
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
