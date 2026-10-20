import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

/** 页面间距（px），与 reader.css 中 .reader-track 的 gap 保持一致 */
const PAGE_GAP = 10;
/** 视口上方固定多渲染的页数（快速向上滚回时缓冲） */
const BUFFER_PAGES_ABOVE = 1;
/** 视口下方多渲染的屏幕倍数（快速下滑时缓冲） */
const BUFFER_SCREENS_BELOW = 1.5;
/** 未测量页高的估计：容器宽度 × 该比例（漫画以竖长页型为主） */
const ESTIMATE_ASPECT = 1.5;
const ESTIMATE_MIN = 240;

export interface PageRange {
  /** 区间起始页下标（含） */
  start: number;
  /** 区间结束页下标（不含） */
  end: number;
}

interface Deps {
  /** 阅读器滚动容器 */
  scrollRef: React.RefObject<HTMLDivElement>;
  /** 总页数；变化视为打开了新漫画，重置布局 */
  pageCount: number;
  /** 缩放比例：1 = 宽度铺满容器 */
  zoom: number;
  /** 进入时恢复到的阅读进度 0-1 */
  progress: number;
  /** 节流保存进度 */
  onSaveProgress: (fraction: number) => void;
}

export interface ReaderWindow {
  /** 需要真实渲染（挂 <img>）的页下标范围 [start, end) */
  range: PageRange;
  /** 当前页码（1 起） */
  current: number;
  /** 阅读进度 0-1 */
  fraction: number;
  /** 最新进度（退出时保存用，避免 state 过期） */
  fractionRef: React.MutableRefObject<number>;
  /** 每页实测高度（px）；0 = 未测量，渲染时回退估计值 */
  heights: number[];
  /** 估计页高（px） */
  estimate: number;
  /** 窗口上方占位高度（px） */
  spacerTop: number;
  /** 窗口下方占位高度（px） */
  spacerBottom: number;
  /** 绑定到滚动容器的 onScroll */
  onScroll: () => void;
  /** 图片加载完成后回报实测高度（用于校正布局） */
  measure: (index: number, height: number) => void;
  /** 拖动进度滑块跳转 */
  jumpTo: (fraction: number) => void;
}

/** 阅读器懒加载：窗口化渲染。
 *
 *  背景：此前一部漫画的全部图片会一次性挂进 DOM，WebView 为每张图
 *  解码原图（扫描漫画常见 3000×4500、单张数 MB），数百页的漫画内存
 *  直接撑爆进程。这里只把视口附近（含上下缓冲）的图片挂成 <img>，
 *  其余页用等高的占位 div 撑出正确的滚动高度与进度比例——
 *  滚动条长度、页码、进度恢复都不受影响，DOM 里的图片则始终只有几张。
 *
 *  高度模型：页高未测量时按「容器宽度 × 1.5」估计；图片 onLoad 后
 *  实测高度写回。测量值在缩放 / 旋屏（宽度变化）时按比例换算，
 *  图片宽高比不变，换算结果与重新实测一致。
 *  每次布局变化先记录「锚点」（视口顶部所在页 + 页内偏移），
 *  重排后把锚点恢复回去，避免内容跳动。 */
export function useReaderWindow({
  scrollRef,
  pageCount,
  zoom,
  progress,
  onSaveProgress,
}: Deps): ReaderWindow {
  const [heights, setHeights] = useState<number[]>([]);
  const [range, setRange] = useState<PageRange>({ start: 0, end: 0 });
  const [current, setCurrent] = useState(1);
  const [fraction, setFraction] = useState(progress);
  const [estimate, setEstimate] = useState(ESTIMATE_MIN);

  const heightsRef = useRef<number[]>([]);
  /** prefix[i] = 第 i 页顶部的偏移；prefix[n] = 内容总高（含每页后的间距） */
  const prefixRef = useRef<number[]>([0]);
  const rafRef = useRef(0);
  const restoreDoneRef = useRef(false);
  const lastSaveRef = useRef(0);
  const zoomRef = useRef(zoom);
  /** 最近一次布局时的页面宽度（含缩放），用于检测宽度变化 */
  const widthRef = useRef(0);
  const fractionRef = useRef(progress);
  const saveRef = useRef(onSaveProgress);
  saveRef.current = onSaveProgress;
  /** 待恢复的锚点： heights 提交到 DOM 后、绘制前应用 */
  const pendingAnchorRef = useRef<{ index: number; offset: number } | null>(null);

  /** 由偏移量找页下标：最后满足 prefix[i] <= offset 的 i（二分） */
  const indexAt = useCallback((offset: number): number => {
    const prefix = prefixRef.current;
    let lo = 0;
    let hi = prefix.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (prefix[mid] <= offset) lo = mid;
      else hi = mid - 1;
    }
    return lo;
  }, []);

  /** 记录当前锚点（视口顶部所在页 + 页内偏移） */
  const captureAnchor = useCallback(():
    | { index: number; offset: number }
    | null => {
    const el = scrollRef.current;
    if (!el) return null;
    const index = indexAt(el.scrollTop);
    return { index, offset: el.scrollTop - prefixRef.current[index] };
  }, [scrollRef, indexAt]);

  /** 重算布局：宽度变化时按比例换算已测高度，再重算每页累计偏移 */
  const relayout = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const width = Math.max(1, el.clientWidth) * zoomRef.current;
    if (widthRef.current > 0 && Math.abs(width - widthRef.current) > 0.5) {
      const ratio = width / widthRef.current;
      for (let i = 0; i < heightsRef.current.length; i += 1) {
        const h = heightsRef.current[i];
        if (h > 0) heightsRef.current[i] = Math.round(h * ratio);
      }
    }
    widthRef.current = width;
    const est = Math.max(ESTIMATE_MIN, Math.round(width * ESTIMATE_ASPECT));
    setEstimate(est);
    const n = heightsRef.current.length;
    const prefix = new Array<number>(n + 1);
    prefix[0] = 0;
    for (let i = 0; i < n; i += 1) {
      prefix[i + 1] = prefix[i] + (heightsRef.current[i] || est) + PAGE_GAP;
    }
    prefixRef.current = prefix;
  }, [scrollRef]);

  /** 依据滚动位置刷新渲染窗口 / 当前页 / 进度（含节流保存） */
  const updateWindow = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const prefix = prefixRef.current;
    const n = prefix.length - 1;
    if (n === 0) return;
    const top = el.scrollTop;
    const view = el.clientHeight;
    // 滚到底部时中线可能落在末页之后的底部留白里，indexAt 会返回 n，
    // 这里夹回最后一页，页码不能出现 n+1
    const anchor = Math.min(n - 1, indexAt(top));
    const start = Math.max(0, anchor - BUFFER_PAGES_ABOVE);
    let end = anchor + 1;
    const limit = top + view * (1 + BUFFER_SCREENS_BELOW);
    while (end < n && prefix[end] < limit) end += 1;
    end = Math.min(n, end + 1);
    setRange((prev) =>
      prev.start === start && prev.end === end ? prev : { start, end }
    );

    setCurrent(Math.min(n, indexAt(top + view / 2) + 1));

    const max = el.scrollHeight - el.clientHeight;
    const f = max > 0 ? Math.min(1, Math.max(0, top / max)) : 0;
    fractionRef.current = f;
    setFraction(f);
    const now = performance.now();
    if (now - lastSaveRef.current > 400) {
      lastSaveRef.current = now;
      saveRef.current(f);
    }
  }, [scrollRef, indexAt]);

  /** 滚动事件：rAF 合帧，避免每个 touchmove 都重算 */
  const onScroll = useCallback(() => {
    if (rafRef.current) return;
    rafRef.current = requestAnimationFrame(() => {
      rafRef.current = 0;
      updateWindow();
    });
  }, [updateWindow]);

  const measure = useCallback(
    (index: number, height: number) => {
      const h = Math.round(height);
      const arr = heightsRef.current;
      if (index < 0 || index >= arr.length || arr[index] === h) return;
      const anchor = captureAnchor();
      arr[index] = h;
      relayout();
      // 锚点恢复放到 layout effect：等 heights 提交进 DOM、
      // scrollHeight 反映新布局后再设 scrollTop，否则会被旧高度裁掉
      if (anchor) pendingAnchorRef.current = anchor;
      setHeights([...arr]);
    },
    [captureAnchor, relayout]
  );

  /* ---------- heights 提交后、绘制前：恢复锚点并刷新窗口 ---------- */
  useLayoutEffect(() => {
    const el = scrollRef.current;
    const anchor = pendingAnchorRef.current;
    if (el && anchor) {
      pendingAnchorRef.current = null;
      const max = el.scrollHeight - el.clientHeight;
      el.scrollTop = Math.min(max, Math.max(0, prefixRef.current[anchor.index] + anchor.offset));
    }
    updateWindow();
  }, [heights, scrollRef, updateWindow]);

  /* ---------- 打开新漫画：重置布局，恢复上次阅读位置 ---------- */
  useEffect(() => {
    heightsRef.current = new Array(pageCount).fill(0);
    prefixRef.current = [0];
    restoreDoneRef.current = false;
    lastSaveRef.current = 0;
    fractionRef.current = progress;
    setHeights([]);
    setCurrent(1);
    setFraction(progress);
    setRange({ start: 0, end: 0 });
    if (pageCount === 0) return;

    // 首帧：按估计高度布好占位，算出初始窗口
    relayout();
    updateWindow();

    // 恢复进度要等占位 DOM 真正提交（双 rAF 确保 React 已渲染）：
    // 此时 scrollHeight 才反映估计高度，直接设 scrollTop 会被裁成 0。
    // 落在「估计页码」上即可，后续实测高度会按锚点自动校正。
    let inner = 0;
    const restore = () => {
      const el = scrollRef.current;
      if (!el || restoreDoneRef.current) return;
      restoreDoneRef.current = true;
      const p = Math.min(1, Math.max(0, progress));
      if (p > 0.001 && pageCount > 1) {
        el.scrollTop = prefixRef.current[Math.round(p * (pageCount - 1))];
      }
      updateWindow();
    };
    const outer = requestAnimationFrame(() => {
      inner = requestAnimationFrame(restore);
    });
    return () => {
      cancelAnimationFrame(outer);
      cancelAnimationFrame(inner);
    };
    // 注意：progress 刻意不进依赖——进度变化不应重置阅读器，
    // 只在 pageCount 变化（打开新漫画）时恢复一次
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pageCount]);

  /* ---------- 缩放变化：高度按比例换算，锚点不动 ---------- */
  useEffect(() => {
    if (zoomRef.current === zoom) return;
    const anchor = captureAnchor();
    zoomRef.current = zoom;
    relayout(); // 内部按新旧宽度比换算 heights
    if (anchor) pendingAnchorRef.current = anchor;
    setHeights([...heightsRef.current]);
  }, [zoom, captureAnchor, relayout]);

  /* ---------- 窗口尺寸变化（旋屏 / 折叠屏形态切换）：同上 ---------- */
  useEffect(() => {
    const onResize = () => {
      const anchor = captureAnchor();
      relayout();
      if (anchor) pendingAnchorRef.current = anchor;
      setHeights([...heightsRef.current]);
    };
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [captureAnchor, relayout]);

  /** 卸载时取消未执行的 rAF */
  useEffect(() => {
    return () => {
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, []);

  const jumpTo = useCallback(
    (f: number) => {
      const el = scrollRef.current;
      if (!el) return;
      const max = el.scrollHeight - el.clientHeight;
      el.scrollTop = Math.min(max, Math.max(0, f * max));
    },
    [scrollRef]
  );

  /* ---------- 占位高度：把窗口之外的页换算成上下两块等高的占位 ---------- */
  const prefix = prefixRef.current;
  const n = prefix.length - 1;
  const spacerTop =
    range.start > 0 ? Math.max(0, prefix[range.start] - PAGE_GAP) : 0;
  const spacerBottom =
    range.end < n ? Math.max(0, prefix[n] - prefix[range.end] - PAGE_GAP) : 0;

  return {
    range,
    current,
    fraction,
    fractionRef,
    heights,
    estimate,
    spacerTop,
    spacerBottom,
    onScroll,
    measure,
    jumpTo,
  };
}
