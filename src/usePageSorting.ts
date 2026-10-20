import { useEffect, useRef, useState } from "react";

const LONG_PRESS_MS = 480;
const SCROLL_TOLERANCE = 12; // 超过视为滚动，取消长按
const DRAG_TOLERANCE = 10; // 长按后超过视为开始拖动

/** 触屏长按手势状态 */
interface TouchState {
  id: number;
  startX: number;
  startY: number;
  timer: number;
  /** 长按已触发（震动提示过） */
  fired: boolean;
  /** 已进入拖动排序 */
  dragging: boolean;
  page: string | null;
  thumbEl: HTMLElement | null;
  ghost: HTMLElement | null;
  offsetX: number;
  offsetY: number;
  /** 当前插入下标（存在手势状态里，避免依赖可能滞后的 React state） */
  ins: number | null;
}

export interface PageSorting {
  /** 拖动中的页面路径 */
  dragPage: string | null;
  /** 插入指示：将落在第几个缩略图之前（等于数量表示末尾）；null = 无拖动 */
  overIndex: number | null;
  /** 网格容器上绑定（dragover / drop） */
  containerHandlers: {
    onDragOver: (e: React.DragEvent) => void;
    onDrop: (e: React.DragEvent) => void;
  };
  /** 每个缩略图上绑定 */
  thumbHandlers: {
    draggable: boolean;
    onDragStart: (e: React.DragEvent) => void;
    onDragEnd: () => void;
  };
}

/** 把 page 从 pages 移到插入下标 ins 处；位置没变返回 null */
function movePage(pages: string[], page: string, ins: number): string[] | null {
  const from = pages.indexOf(page);
  if (from < 0) return null;
  // 先摘掉自己再算目标位：插在自己前面时 adjusted === from，等于没动
  const adjusted = ins <= from ? ins : ins - 1;
  const next = pages.filter((p) => p !== page);
  next.splice(Math.max(0, Math.min(adjusted, next.length)), 0, page);
  return next.join("\u0000") === pages.join("\u0000") ? null : next;
}

/** 漫画图片排序：鼠标 HTML5 拖放 + 触屏长按拖动。
 *
 *  与书架的 useItemSorting 同一套手势，但面向「一部漫画内的图片」这种
 *  扁平列表：落点用插入下标表示（落在第几张图之前），不做文件夹移入。
 *  缩略图需带 data-page 属性；内部的删除按钮等加 data-no-drag 避免误触拖动。 */
export function usePageSorting(
  containerRef: React.RefObject<HTMLDivElement>,
  pages: string[],
  onReorder: (next: string[]) => void | Promise<void>,
): PageSorting {
  const [dragPage, setDragPage] = useState<string | null>(null);
  const [overIndex, setOverIndex] = useState<number | null>(null);
  const touchRef = useRef<TouchState | null>(null);
  // 触屏监听是原生绑定、重挂时机可能滞后于渲染，用时从 ref 取最新值
  const pagesRef = useRef(pages);
  pagesRef.current = pages;
  const onReorderRef = useRef(onReorder);
  onReorderRef.current = onReorder;

  /** 根据指针位置算插入下标：插在第几个缩略图之前 */
  function insertionIndexAt(x: number, y: number): number {
    const container = containerRef.current;
    if (!container) return 0;
    const thumbs = Array.from(container.querySelectorAll<HTMLElement>("[data-page]"));
    for (let i = 0; i < thumbs.length; i += 1) {
      const r = thumbs[i].getBoundingClientRect();
      if (y < r.top) return i; // 指针在这一行的上方
      if (y > r.bottom) continue; // 在这一行下方，继续找
      return x < r.left + r.width / 2 ? i : i + 1; // 同一行按左右分前后
    }
    return thumbs.length;
  }

  function clearDrag() {
    setDragPage(null);
    setOverIndex(null);
  }

  /* ---------- 鼠标拖放 ---------- */

  function onDragStart(e: React.DragEvent) {
    const page = (e.currentTarget as HTMLElement).dataset.page;
    if (!page) return;
    setDragPage(page);
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", page);
  }

  function onDragEnd() {
    clearDrag();
  }

  function onDragOver(e: React.DragEvent) {
    if (!dragPage) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
    setOverIndex(insertionIndexAt(e.clientX, e.clientY));
  }

  async function onDrop(e: React.DragEvent) {
    if (!dragPage) return;
    e.preventDefault();
    const next = movePage(pages, dragPage, insertionIndexAt(e.clientX, e.clientY));
    clearDrag();
    if (next) await onReorder(next);
  }

  /* ---------- 触屏：长按后拖动排序 ---------- */

  function startTouchDrag(st: TouchState, t: Touch) {
    const el = st.thumbEl;
    if (!el || !st.page) return;
    const rect = el.getBoundingClientRect();
    const ghost = el.cloneNode(true) as HTMLElement;
    ghost.classList.add("page-drag-ghost");
    ghost.style.width = `${rect.width}px`;
    document.body.appendChild(ghost);
    st.ghost = ghost;
    st.offsetX = t.clientX - rect.left;
    st.offsetY = t.clientY - rect.top;
    el.classList.add("page-thumb-dragging");
    setDragPage(st.page);
    setOverIndex(null);
    st.ins = null;
    // 立即定位一次，避免闪烁
    ghost.style.transform = `translate(${t.clientX - st.offsetX}px, ${
      t.clientY - st.offsetY
    }px)`;
  }

  function updateTouchDrag(st: TouchState, t: Touch) {
    if (st.ghost) {
      st.ghost.style.transform = `translate(${t.clientX - st.offsetX}px, ${
        t.clientY - st.offsetY
      }px)`;
    }
    st.ins = insertionIndexAt(t.clientX, t.clientY);
    setOverIndex(st.ins);
  }

  function cleanupTouchDrag(st: TouchState) {
    window.clearTimeout(st.timer);
    st.ghost?.remove();
    st.ghost = null;
    st.thumbEl?.classList.remove("page-thumb-dragging");
    clearDrag();
  }

  async function endTouchDrag(st: TouchState) {
    const page = st.page;
    const ins = st.ins;
    cleanupTouchDrag(st);
    if (!page || ins === null) return;
    const next = movePage(pagesRef.current, page, ins);
    if (next) await onReorderRef.current(next);
  }

  // 原生监听：React 的 onTouchMove 是 passive 的，无法 preventDefault 阻止滚动
  // （不依赖数组：每次渲染重新绑定，pages / onReorder 另用 ref 保证最新）
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const onStart = (e: TouchEvent) => {
      if (e.touches.length !== 1) return;
      const target = e.target as HTMLElement | null;
      // 删除按钮等不参与拖动
      if (target?.closest("[data-no-drag]")) return;
      const el = target?.closest<HTMLElement>("[data-page]");
      if (!el) return;
      const t = e.touches[0];
      const st: TouchState = {
        id: t.identifier,
        startX: t.clientX,
        startY: t.clientY,
        timer: 0,
        fired: false,
        dragging: false,
        page: el.dataset.page ?? null,
        thumbEl: el,
        ghost: null,
        offsetX: 0,
        offsetY: 0,
        ins: null,
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
        e.preventDefault(); // 阻止合成 click（否则会改到封面）
        void endTouchDrag(st);
      } else if (st.fired) {
        // 长按但没有拖动：吞掉 click，不改变封面
        e.preventDefault();
      }
    };

    const onCancel = () => {
      const st = touchRef.current;
      touchRef.current = null;
      if (!st) return;
      if (st.dragging) cleanupTouchDrag(st);
      else window.clearTimeout(st.timer);
    };

    container.addEventListener("touchstart", onStart, { passive: true });
    container.addEventListener("touchmove", onMove, { passive: false });
    container.addEventListener("touchend", onEnd, { passive: false });
    container.addEventListener("touchcancel", onCancel, { passive: true });
    return () => {
      container.removeEventListener("touchstart", onStart);
      container.removeEventListener("touchmove", onMove);
      container.removeEventListener("touchend", onEnd);
      container.removeEventListener("touchcancel", onCancel);
    };
  });

  return {
    dragPage,
    overIndex,
    containerHandlers: { onDragOver, onDrop },
    thumbHandlers: { draggable: true, onDragStart, onDragEnd },
  };
}
