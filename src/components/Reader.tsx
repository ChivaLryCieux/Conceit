import { useCallback, useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { fetchPages, saveProgress } from "../api";
import { MinusIcon, PlusIcon } from "./icons";
import type { Item } from "../types";
import type { Toast } from "../App";

interface Props {
  item: Item;
  onExit: () => void;
  onToast: (text: string, tone?: Toast["tone"]) => void;
}

const ZOOM_MIN = 0.5;
const ZOOM_MAX = 3;

function clampZoom(z: number): number {
  return Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Math.round(z * 100) / 100));
}

/** 阅读器：竖向拼接图片上下滑动；点击画面显隐唯一顶部框栏 */
export default function Reader({ item, onExit, onToast }: Props) {
  const [pages, setPages] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadedCount, setLoadedCount] = useState(0);
  const [chromeVisible, setChromeVisible] = useState(true);
  const [current, setCurrent] = useState(1);
  const [fraction, setFraction] = useState(item.manga?.progress ?? 0);
  /** 缩放比例：1 = 宽度填充整个屏幕 */
  const [zoom, setZoom] = useState(1);

  const scrollRef = useRef<HTMLDivElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const fractionRef = useRef(item.manga?.progress ?? 0);
  const lastSaveRef = useRef(0);
  const restoreDoneRef = useRef(false);

  /* ---------- 加载图片 ---------- */
  useEffect(() => {
    let alive = true;
    setLoading(true);
    setLoadedCount(0);
    restoreDoneRef.current = false;
    void fetchPages(item.id)
      .then((list) => {
        if (!alive) return;
        setPages(list);
        setLoading(false);
      })
      .catch((e) => {
        if (!alive) return;
        setLoading(false);
        onToast(`读取图片失败：${e}`, "error");
      });
    return () => {
      alive = false;
    };
  }, [item.id, onToast]);

  /* ---------- 退出时保存进度 ---------- */
  useEffect(() => {
    return () => {
      void saveProgress(item.id, fractionRef.current).catch(() => {});
    };
  }, [item.id]);

  /* ---------- 滚动：页码、进度、节流保存 ---------- */
  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const max = el.scrollHeight - el.clientHeight;
    const f = max > 0 ? Math.min(1, Math.max(0, el.scrollTop / max)) : 0;
    fractionRef.current = f;
    setFraction(f);

    // 当前页码：视口中线之前最后一张图
    const mid = el.scrollTop + el.clientHeight / 2;
    const imgs = Array.from(
      el.querySelectorAll<HTMLImageElement>(".reader-page")
    );
    let idx = 0;
    for (let i = 0; i < imgs.length; i += 1) {
      if (imgs[i].offsetTop <= mid) idx = i;
    }
    setCurrent(Math.min(idx + 1, Math.max(imgs.length, 1)));

    // 节流持久化
    const now = performance.now();
    if (now - lastSaveRef.current > 400) {
      lastSaveRef.current = now;
      void saveProgress(item.id, f).catch(() => {});
    }
  }, [item.id]);

  /* ---------- 恢复上次阅读位置 ---------- */
  useEffect(() => {
    if (restoreDoneRef.current || pages.length === 0) return;
    if (loadedCount < pages.length) return; // 等图片就位，保证滚动高度准确
    const el = scrollRef.current;
    if (!el) return;
    restoreDoneRef.current = true;
    const p = Math.min(1, Math.max(0, item.manga?.progress ?? 0));
    if (p <= 0.001) return;
    const max = el.scrollHeight - el.clientHeight;
    el.scrollTop = p * max;
    handleScroll();
  }, [pages.length, loadedCount, item.manga?.progress, handleScroll]);

  /* ---------- 键盘操作 ---------- */
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onExit();
        return;
      }
      const el = scrollRef.current;
      if (!el) return;
      const step = el.clientHeight * 0.85;
      switch (e.key) {
        case "ArrowDown":
        case "PageDown":
        case " ":
          e.preventDefault();
          el.scrollBy({ top: step });
          break;
        case "ArrowUp":
        case "PageUp":
          e.preventDefault();
          el.scrollBy({ top: -step });
          break;
        case "Home":
          e.preventDefault();
          el.scrollTo({ top: 0 });
          break;
        case "End":
          e.preventDefault();
          el.scrollTo({ top: el.scrollHeight });
          break;
        default:
          break;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onExit]);

  /* ---------- Ctrl + 滚轮缩放（需非 passive 监听才能阻止浏览器默认缩放） ---------- */
  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey) return;
      e.preventDefault();
      // 触控板捏合 / 鼠标滚轮都会带 ctrlKey；按增量做乘性缩放，手感平滑
      const d = e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY;
      setZoom((z) => clampZoom(z * Math.exp(-d * 0.0015)));
      setChromeVisible(true); // 缩放时显出框栏，方便看到当前比例
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  const jumpTo = (f: number) => {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTo({ top: f * (el.scrollHeight - el.clientHeight) });
  };

  const zoomed = zoom > 1.001;

  return (
    <div className="reader" ref={rootRef}>
      <div
        className={`reader-topbar${chromeVisible ? "" : " chrome-hidden"}`}
        onClick={(e) => e.stopPropagation()}
      >
        <button type="button" className="btn" onClick={onExit}>
          ← 退出阅读
        </button>
        <span className="reader-title">{item.title}</span>

        <div className="reader-zoom" onClick={(e) => e.stopPropagation()}>
          <button
            type="button"
            className="btn btn-icon"
            title="缩小"
            aria-label="缩小"
            disabled={zoom <= ZOOM_MIN}
            onClick={() => setZoom((z) => clampZoom(z - 0.1))}
          >
            <MinusIcon />
          </button>
          <button
            type="button"
            className="reader-zoom-value"
            title="恢复适应宽度"
            onClick={() => setZoom(1)}
          >
            {Math.round(zoom * 100)}%
          </button>
          <button
            type="button"
            className="btn btn-icon"
            title="放大"
            aria-label="放大"
            disabled={zoom >= ZOOM_MAX}
            onClick={() => setZoom((z) => clampZoom(z + 0.1))}
          >
            <PlusIcon />
          </button>
        </div>

        <input
          className="reader-slider"
          type="range"
          min={0}
          max={1000}
          value={Math.round(fraction * 1000)}
          onChange={(e) => jumpTo(Number(e.target.value) / 1000)}
          onClick={(e) => e.stopPropagation()}
          aria-label="阅读进度"
        />
        <span className="reader-counter">
          {pages.length > 0
            ? `${current} / ${pages.length} · ${Math.round(fraction * 100)}%`
            : "—"}
        </span>
      </div>

      <div
        className="reader-scroll"
        ref={scrollRef}
        onScroll={handleScroll}
        onClick={() => setChromeVisible((v) => !v)}
      >
        {loading ? (
          <div className="reader-loading">正在加载图片…</div>
        ) : pages.length === 0 ? (
          <div className="reader-loading">这部漫画还没有图片</div>
        ) : (
          <div className={`reader-track${zoomed ? " reader-track-zoomed" : ""}`}>
            {pages.map((p, i) => (
              <img
                key={p}
                className="reader-page"
                src={convertFileSrc(p)}
                alt={`第 ${i + 1} 页`}
                style={{ width: `${zoom * 100}%` }}
                draggable={false}
                onLoad={() => setLoadedCount((c) => c + 1)}
                onError={() => setLoadedCount((c) => c + 1)}
              />
            ))}
          </div>
        )}
      </div>

      {chromeVisible && pages.length > 0 && (
        <div className="reader-hint">
          点击画面显隐工具栏 · Ctrl+滚轮缩放 · 滚轮 / 方向键翻页 · Esc 退出
        </div>
      )}
    </div>
  );
}
