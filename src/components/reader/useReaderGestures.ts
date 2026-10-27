import { useCallback, useEffect, useRef } from "react";
import { clampZoom, isZoomed } from "./zoom";

/** 位移小于它算"点击"而非拖动（px） */
const TAP_MOVE_TOLERANCE = 10;
/** 两次点击间隔小于它算"双击"（ms） */
const DOUBLE_TAP_INTERVAL = 300;
/** 两次点击落点相距小于它才算同一点（px），防止快速连点不同位置误触发 */
const DOUBLE_TAP_SLOP = 40;

interface Deps {
  /** 阅读器滚动容器：竖向滚动交给浏览器，横向平移与捏合由本 hook 接管 */
  scrollRef: React.RefObject<HTMLDivElement>;
  /** 当前缩放比例；每次渲染同步进 ref，事件回调里读到的永不过期 */
  zoom: number;
  /** 缩放变化（受控 state） */
  onZoom: (z: number) => void;
  /** 单击画面（无位移）：显隐底栏 */
  onTap: () => void;
  /** 双击还原：缩放回 1 */
  onReset: () => void;
}

export interface ReaderGestures {
  onPointerDown: (e: React.PointerEvent<HTMLDivElement>) => void;
  onPointerMove: (e: React.PointerEvent<HTMLDivElement>) => void;
  onPointerUp: (e: React.PointerEvent<HTMLDivElement>) => void;
  onPointerCancel: (e: React.PointerEvent<HTMLDivElement>) => void;
  onClick: (e: React.MouseEvent<HTMLDivElement>) => void;
}

/** 阅读器触控手势：双指捏合缩放、放大后单指平移、双击还原。
 *
 *  为什么自己接：viewport 是 user-scalable=no——漫画阅读要的是应用内受控
 *  缩放（有上下限、要能被底栏的百分比反映），不是浏览器整页缩放；而 Ctrl+
 *  滚轮只有桌面触控板 / 鼠标能用，手机上此前只剩底栏两个小按钮，看漫画时
 *  没人想一直点。这里用 Pointer Events 把捏合接回来。
 *
 *  与原生滚动的分工：.reader-scroll 的 touch-action 是 pan-y——单指竖滑
 *  交给浏览器（带动量，手感最好），捏合与横平移浏览器不管，由这里处理。
 *  竖滑一旦开始浏览器会 pointercancel 掉那根手指，所以"先滑着再加一指"
 *  接不上，先落两根手指再捏即可——与其他阅读器一致。
 *
 *  另两处顺手覆盖的：鼠标在放大状态下可拖拽平移；双击（触屏或鼠标）还原。
 *  不做任何 UI 提示，手势本身即是全部交互。 */
export function useReaderGestures({
  scrollRef,
  zoom,
  onZoom,
  onTap,
  onReset,
}: Deps): ReaderGestures {
  const zoomRef = useRef(zoom);
  zoomRef.current = zoom;
  const onZoomRef = useRef(onZoom);
  onZoomRef.current = onZoom;
  const onTapRef = useRef(onTap);
  onTapRef.current = onTap;
  const onResetRef = useRef(onReset);
  onResetRef.current = onReset;

  /** 活动指针：pointerId → 最新客户端坐标 */
  const pointers = useRef(new Map<number, { x: number; y: number }>());
  /** 单指手势的最新坐标（位移判定 + 横平移基准） */
  const single = useRef<{ x: number; y: number } | null>(null);
  /** 捏合会话：落下第二指时的双指距 / 缩放 / 中点，捏合按比例从它起算 */
  const pinch = useRef<{
    dist: number;
    zoom: number;
    midX: number;
    midY: number;
  } | null>(null);
  /** 本次手势是否产生过位移；位移后 pointerup 补发的 click 必须吞掉 */
  const moved = useRef(false);
  /** 上一次单击（双击判定用） */
  const lastTap = useRef<{ time: number; x: number; y: number } | null>(null);
  /** 待执行的捏合帧：双指各发一次 pointermove，同一帧里只算一次 */
  const rafRef = useRef(0);

  const clear = useCallback(() => {
    if (rafRef.current) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = 0;
    }
    pointers.current.clear();
    single.current = null;
    pinch.current = null;
  }, []);

  // 卸载时清掉指针表：阅读器卸载瞬间若有手指还按着，别把状态留给下一次
  useEffect(() => clear, [clear]);

  /** 执行一帧捏合：距离比 → 缩放，中点位移 → 平移。
   *  必须合并到 rAF 里算：一次 touchmove 会派发两根手指各一次 pointermove，
   *  逐事件算就会用到另一根手指的旧坐标（双指同向平移时算出的距离会偏小，
   *  缩放瞬间抽一截再弹回）。合并后每帧用两根手指的最新坐标算一次。
   *  另外不能做"值没变就不 setZoom"的去重：批量更新下去重拿已提交的值
   *  比较，会把"回到原值"的那次更新吞掉，缩放凝固在中间值上。 */
  const applyPinch = useCallback(() => {
    rafRef.current = 0;
    const el = scrollRef.current;
    const p = pinch.current;
    if (!el || !p || pointers.current.size < 2) return;
    const [a, b] = [...pointers.current.values()];
    const midX = (a.x + b.x) / 2;
    const midY = (a.y + b.y) / 2;
    // 缩放：双指距离比 × 起始比例，乘性变化才跟手
    onZoomRef.current(
      clampZoom((p.zoom * Math.hypot(a.x - b.x, a.y - b.y)) / p.dist)
    );
    // 平移：内容跟着双指中点走，捏合焦点才不会飘
    el.scrollLeft -= midX - p.midX;
    el.scrollTop -= midY - p.midY;
    p.midX = midX;
    p.midY = midY;
    moved.current = true;
  }, [scrollRef]);

  const schedulePinch = useCallback(() => {
    if (rafRef.current) return;
    rafRef.current = requestAnimationFrame(applyPinch);
  }, [applyPinch]);

  const onPointerDown = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    if (e.pointerType === "mouse" && e.button !== 0) return;
    // 显式捕获到滚动容器上：捏合会改变页高、窗口化渲染随之挂载 / 卸载 <img>，
    // 不捕获的话手指底下那张图一被换掉事件流就断了
    try {
      e.currentTarget.setPointerCapture(e.pointerId);
    } catch {
      // 指针已收回等情况下捕获会抛，忽略即可（地图里仍有记录）
    }
    moved.current = false; // 新手势重新计位移
    pointers.current.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pointers.current.size === 2) {
      const [a, b] = [...pointers.current.values()];
      const d = Math.hypot(a.x - b.x, a.y - b.y);
      if (d > 0) {
        pinch.current = {
          dist: d,
          zoom: zoomRef.current,
          midX: (a.x + b.x) / 2,
          midY: (a.y + b.y) / 2,
        };
        single.current = null;
      }
    } else if (pointers.current.size === 1) {
      single.current = { x: e.clientX, y: e.clientY };
    }
  }, []);

  const onPointerMove = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      if (!pointers.current.has(e.pointerId)) return;
      pointers.current.set(e.pointerId, { x: e.clientX, y: e.clientY });

      if (pinch.current && pointers.current.size >= 2) {
        schedulePinch();
        return;
      }

      const el = scrollRef.current;
      if (!el) return;
      const s = single.current;
      if (pointers.current.size === 1 && s) {
        const dx = e.clientX - s.x;
        const dy = e.clientY - s.y;
        if (Math.abs(dx) > TAP_MOVE_TOLERANCE || Math.abs(dy) > TAP_MOVE_TOLERANCE) {
          moved.current = true;
        }
        // 只有放大后才需要手动横平移：内容比屏幕宽，而 touch-action 是
        // pan-y，横向浏览器不管。未放大时内容恰好铺满，横向没有可滚的量
        if (isZoomed(zoomRef.current)) {
          el.scrollLeft -= dx;
          el.scrollTop -= dy;
          single.current = { x: e.clientX, y: e.clientY };
        }
      }
    },
    [scrollRef]
  );

  const onPointerUp = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    pointers.current.delete(e.pointerId);
    if (pointers.current.size < 2) pinch.current = null;
    if (pointers.current.size === 1) {
      // 捏合中抬起一指：剩下那指无缝接成平移，基准取它当前所在位置
      const [a] = [...pointers.current.values()];
      single.current = { x: a.x, y: a.y };
      return;
    }
    if (pointers.current.size > 0) return;
    single.current = null;
    if (moved.current) return; // 捏合 / 拖动结束，不是点击

    // 单击 / 双击判定：时间与落点都要近。双击的语义是"还原"，
    // 不弹任何提示；随之而来的两次 click 互相抵消，底栏显隐不变
    const now = e.timeStamp;
    const last = lastTap.current;
    if (
      last &&
      now - last.time < DOUBLE_TAP_INTERVAL &&
      Math.hypot(e.clientX - last.x, e.clientY - last.y) < DOUBLE_TAP_SLOP
    ) {
      lastTap.current = null;
      onResetRef.current();
    } else {
      lastTap.current = { time: now, x: e.clientX, y: e.clientY };
    }
  }, []);

  const onPointerCancel = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    // 浏览器接管手势（原生竖滚启动）时 cancel：本次手势作废，
    // 标记位移，随后的 click 不能当成点击显隐底栏
    pointers.current.delete(e.pointerId);
    if (pointers.current.size === 0) {
      single.current = null;
      pinch.current = null;
      moved.current = true;
    } else if (pointers.current.size < 2) {
      pinch.current = null;
    }
  }, []);

  const onClick = useCallback((e: React.MouseEvent<HTMLDivElement>) => {
    if (moved.current) {
      moved.current = false;
      e.preventDefault();
      return;
    }
    onTapRef.current();
  }, []);

  return { onPointerDown, onPointerMove, onPointerUp, onPointerCancel, onClick };
}
