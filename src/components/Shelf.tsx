import { useEffect, useRef, useState } from "react";
import Card, { type DropMode } from "./Card";
import ContextMenu, { type MenuAction } from "./ContextMenu";
import InputDialog from "./InputDialog";
import ItemSettings from "./ItemSettings";
import { BookLogo } from "./icons";
import {
  childrenOf,
  countManga,
  createFolder,
  deleteItems,
  moveItem,
  reorder,
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

/** 触屏长按手势状态 */
interface TouchDragState {
  id: number;
  startX: number;
  startY: number;
  timer: number;
  /** 长按已触发（震动提示过） */
  fired: boolean;
  /** 已进入拖动排序 */
  dragging: boolean;
  cardEl: HTMLElement | null;
  item: Item | null;
  ghost: HTMLElement | null;
  offsetX: number;
  offsetY: number;
}

const LONG_PRESS_MS = 480;
const SCROLL_TOLERANCE = 12; // 超过视为滚动，取消长按
const DRAG_TOLERANCE = 10; // 长按后超过视为开始拖动

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
  /** 新建文件夹弹窗 */
  const [folderDialog, setFolderDialog] = useState(false);
  /** 条目设置弹窗 */
  const [settingsItem, setSettingsItem] = useState<Item | null>(null);
  /** 触屏长按手势状态 */
  const touchRef = useRef<TouchDragState | null>(null);
  const gridRef = useRef<HTMLDivElement>(null);

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

  async function handleCreateFolder(name: string) {
    try {
      await createFolder(name, currentFolder);
      setFolderDialog(false);
      await onReload();
      onToast(`已创建文件夹「${name}」`, "success");
    } catch (e) {
      onToast(`创建文件夹失败：${e}`, "error");
    }
  }

  function handleRename(item: Item) {
    setSettingsItem(item);
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

  /** 根据坐标与命中元素计算落点：文件夹=进入，漫画=前/后，空白=末尾，自己=原位 */
  function computeOverAt(x: number, hit: HTMLElement | null): OverState {
    const el = hit?.closest<HTMLElement>("[data-card-id]") ?? null;
    if (!el) return { id: null, mode: "before" };
    const id = el.dataset.cardId ?? null;
    if (!id) return { id: null, mode: "before" };
    if (id === dragId) return { id, mode: "before" };
    const item = snapshot.index.items.find((i) => i.id === id);
    if (item?.kind === "folder") return { id, mode: "into" };
    const rect = el.getBoundingClientRect();
    const before = x < rect.left + rect.width / 2;
    return { id, mode: before ? "before" : "after" };
  }

  function computeOver(e: React.DragEvent): OverState {
    return computeOverAt(e.clientX, e.target as HTMLElement);
  }

  /** 执行一次排序/移动（鼠标拖放与触屏拖放共用） */
  async function applyDrop(movingId: string, target: OverState) {
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
    await applyDrop(movingId, target);
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

  /* ---------- 触屏：长按打开菜单 / 长按后拖动排序 ---------- */

  /** 创建跟随手指的浮动卡片副本 */
  function startTouchDrag(st: TouchDragState, t: Touch) {
    const el = st.cardEl;
    const item = st.item;
    if (!el || !item) return;
    const rect = el.getBoundingClientRect();
    const ghost = el.cloneNode(true) as HTMLElement;
    ghost.classList.add("card-drag-ghost");
    ghost.style.width = `${rect.width}px`;
    document.body.appendChild(ghost);
    st.ghost = ghost;
    st.offsetX = t.clientX - rect.left;
    st.offsetY = t.clientY - rect.top;
    el.classList.add("card-dragging");
    setDragId(item.id);
    setOver({ id: null, mode: "before" });
    // 立即定位一次，避免闪烁
    ghost.style.transform = `translate(${t.clientX - st.offsetX}px, ${
      t.clientY - st.offsetY
    }px)`;
  }

  function updateTouchDrag(st: TouchDragState, t: Touch) {
    if (st.ghost) {
      st.ghost.style.transform = `translate(${t.clientX - st.offsetX}px, ${
        t.clientY - st.offsetY
      }px)`;
    }
    const hit = document.elementFromPoint(t.clientX, t.clientY) as HTMLElement | null;
    const next = computeOverAt(t.clientX, hit);
    setOver((prev) =>
      prev.id === next.id && prev.mode === next.mode ? prev : next
    );
  }

  function cleanupTouchDrag(st: TouchDragState) {
    window.clearTimeout(st.timer);
    st.ghost?.remove();
    st.ghost = null;
    st.cardEl?.classList.remove("card-dragging");
    setDragId(null);
    setOver({ id: null, mode: "before" });
  }

  async function endTouchDrag(st: TouchDragState) {
    const movingId = st.item?.id;
    const target = over;
    cleanupTouchDrag(st);
    if (!movingId || target.id === movingId) return;
    await applyDrop(movingId, target);
  }

  // 原生监听：React 的 onTouchMove 是 passive 的，无法 preventDefault 阻止滚动
  useEffect(() => {
    const grid = gridRef.current;
    if (!grid) return;

    const onStart = (e: TouchEvent) => {
      if (e.touches.length !== 1) return;
      const t = e.touches[0];
      const el = (t.target as HTMLElement | null)?.closest<HTMLElement>(
        "[data-card-id]"
      );
      if (!el) return;
      const id = el.dataset.cardId ?? null;
      const item = id
        ? snapshot.index.items.find((i) => i.id === id)
        : undefined;
      if (!item) return;
      const st: TouchDragState = {
        id: t.identifier,
        startX: t.clientX,
        startY: t.clientY,
        timer: 0,
        fired: false,
        dragging: false,
        cardEl: el,
        item,
        ghost: null,
        offsetX: 0,
        offsetY: 0,
      };
      st.timer = window.setTimeout(() => {
        st.fired = true;
        if ("vibrate" in navigator) navigator.vibrate(12);
      }, LONG_PRESS_MS);
      touchRef.current = st;
    };

    const onMove = (e: TouchEvent) => {
      const st = touchRef.current;
      if (!st) return;
      if (e.touches.length !== 1) {
        // 多指（捏合等）：放弃当前手势
        if (st.dragging) cleanupTouchDrag(st);
        else window.clearTimeout(st.timer);
        touchRef.current = null;
        return;
      }
      const t = e.touches[0];
      if (t.identifier !== st.id) return;
      const dist = Math.hypot(t.clientX - st.startX, t.clientY - st.startY);
      if (!st.fired) {
        if (dist > SCROLL_TOLERANCE) {
          window.clearTimeout(st.timer);
          touchRef.current = null;
        }
        return;
      }
      if (!st.dragging) {
        if (dist < DRAG_TOLERANCE) return;
        st.dragging = true;
        startTouchDrag(st, t);
      }
      e.preventDefault(); // 阻止页面滚动，交给拖拽
      updateTouchDrag(st, t);
    };

    const onEnd = (e: TouchEvent) => {
      const st = touchRef.current;
      touchRef.current = null;
      if (!st) return;
      window.clearTimeout(st.timer);
      if (st.dragging) {
        e.preventDefault(); // 阻止合成 click
        void endTouchDrag(st);
      } else if (st.fired) {
        e.preventDefault(); // 阻止合成 click（否则会打开漫画/文件夹）
        if (st.item) setMenu({ x: st.startX, y: st.startY, item: st.item });
      }
    };

    const onCancel = () => {
      const st = touchRef.current;
      touchRef.current = null;
      if (!st) return;
      if (st.dragging) cleanupTouchDrag(st);
      else window.clearTimeout(st.timer);
    };

    grid.addEventListener("touchstart", onStart, { passive: true });
    grid.addEventListener("touchmove", onMove, { passive: false });
    grid.addEventListener("touchend", onEnd, { passive: false });
    grid.addEventListener("touchcancel", onCancel, { passive: true });
    return () => {
      grid.removeEventListener("touchstart", onStart);
      grid.removeEventListener("touchmove", onMove);
      grid.removeEventListener("touchend", onEnd);
      grid.removeEventListener("touchcancel", onCancel);
    };
  });

  /* ---------- 渲染 ---------- */

  return (
    <div className="shelf">
      <header className="shelf-header">
        <div className="shelf-brand">
          <span className="brand-mark">
            <BookLogo />
          </span>
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
            onClick={() => setFolderDialog(true)}
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
          ref={gridRef}
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
              onSettings={() => setSettingsItem(item)}
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

      {folderDialog && (
        <InputDialog
          title="新建文件夹"
          label="文件夹名称"
          placeholder="例如：本周更新"
          confirmText="创建"
          onConfirm={(name) => handleCreateFolder(name)}
          onClose={() => setFolderDialog(false)}
        />
      )}

      {settingsItem && (
        <ItemSettings
          item={settingsItem}
          root={snapshot.root}
          onClose={() => setSettingsItem(null)}
          onSaved={() => onReload()}
          onToast={onToast}
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
