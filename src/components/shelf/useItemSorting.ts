import { useEffect, useRef, useState } from "react";
import { reorder } from "../../api";
import type { DropMode } from "./Card";
import type { Item, Toast } from "../../types";

/** 拖放落点指示 */
export interface OverState {
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

export interface SortingDeps {
  /** 当前容器的条目（已按排序排列），用于计算新顺序 */
  items: Item[];
  /** 全部条目，用于按 id 查找 */
  allItems: Item[];
  /** 当前容器；null = 根目录 */
  currentFolder: string | null;
  /** 卡片网格元素（触屏监听挂载点） */
  gridRef: React.RefObject<HTMLDivElement>;
  /** 移动到指定文件夹（面包屑落放复用） */
  onMoveTo: (item: Item, target: string | null) => Promise<void>;
  /** 长按未拖动时打开操作菜单 */
  onLongPress: (x: number, y: number, item: Item) => void;
  onReload: () => Promise<void> | void;
  onToast: (text: string, tone?: Toast["tone"]) => void;
}

/** 书架排序交互：鼠标 HTML5 拖放 + 触屏长按（菜单 / 拖动排序）。
 *  鼠标与触屏共用同一套落点计算与提交逻辑。 */
export function useItemSorting({
  items,
  allItems,
  currentFolder,
  gridRef,
  onMoveTo,
  onLongPress,
  onReload,
  onToast,
}: SortingDeps) {
  /** 拖拽中的条目 id */
  const [dragId, setDragId] = useState<string | null>(null);
  /** 拖放落点指示 */
  const [over, setOver] = useState<OverState>({ id: null, mode: "before" });
  /** 面包屑上的落点（索引） */
  const [crumbOver, setCrumbOver] = useState<number | null>(null);
  /** 触屏长按手势状态 */
  const touchRef = useRef<TouchDragState | null>(null);

  /** 根据坐标与命中元素计算落点：文件夹=进入，漫画=前/后，空白=末尾，自己=原位 */
  function computeOverAt(x: number, hit: HTMLElement | null): OverState {
    const el = hit?.closest<HTMLElement>("[data-card-id]") ?? null;
    if (!el) return { id: null, mode: "before" };
    const id = el.dataset.cardId ?? null;
    if (!id) return { id: null, mode: "before" };
    if (id === dragId) return { id, mode: "before" };
    const item = allItems.find((i) => i.id === id);
    if (item?.kind === "folder") return { id, mode: "into" };
    const rect = el.getBoundingClientRect();
    const before = x < rect.left + rect.width / 2;
    return { id, mode: before ? "before" : "after" };
  }

  /** 执行一次排序/移动（鼠标拖放与触屏拖放共用） */
  async function applyDrop(movingId: string, target: OverState) {
    try {
      if (target.mode === "into" && target.id) {
        const item = allItems.find((i) => i.id === movingId);
        if (item) await onMoveTo(item, target.id);
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

  /* ---------- 鼠标拖放 ---------- */

  function onCardDragStart(e: React.DragEvent, item: Item) {
    setDragId(item.id);
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", item.id);
  }

  function onCardDragEnd() {
    setDragId(null);
    setOver({ id: null, mode: "before" });
  }

  function onGridDragOver(e: React.DragEvent) {
    if (!dragId) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
    setCrumbOver(null);
    const next = computeOverAt(e.clientX, e.target as HTMLElement);
    if (next.id === dragId) return; // 悬停在自己身上时保持当前指示
    setOver(next);
  }

  async function onGridDrop(e: React.DragEvent) {
    if (!dragId) return;
    e.preventDefault();
    const movingId = dragId;
    const target = computeOverAt(e.clientX, e.target as HTMLElement);
    setDragId(null);
    setOver({ id: null, mode: "before" });
    if (target.id === movingId) return;
    await applyDrop(movingId, target);
  }

  /** 卡片上的 drop：阻止冒泡后交给网格统一处理 */
  function onCardDrop(e: React.DragEvent) {
    e.stopPropagation();
    void onGridDrop(e);
  }

  function onCrumbDragOver(e: React.DragEvent, index: number) {
    if (!dragId) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
    setOver({ id: null, mode: "before" });
    setCrumbOver(index);
  }

  function onCrumbDragLeave() {
    setCrumbOver(null);
  }

  async function onCrumbDrop(e: React.DragEvent, folderId: string | null) {
    if (!dragId) return;
    e.preventDefault();
    const movingId = dragId;
    setDragId(null);
    setCrumbOver(null);
    if (movingId === folderId) return;
    const item = allItems.find((i) => i.id === movingId);
    if (item) await onMoveTo(item, folderId);
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
      const item = id ? allItems.find((i) => i.id === id) : undefined;
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
        if (st.item) onLongPress(st.startX, st.startY, st.item);
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

  return {
    dragId,
    over,
    crumbOver,
    handlers: {
      onGridDragOver,
      onGridDrop,
      onCardDragStart,
      onCardDragEnd,
      onCardDrop,
      onCrumbDragOver,
      onCrumbDragLeave,
      onCrumbDrop,
    },
  };
}
