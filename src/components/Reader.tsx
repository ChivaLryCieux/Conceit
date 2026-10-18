import { useCallback, useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { fetchPages, saveProgress } from "../api";
import type { Item } from "../types";
import type { Toast } from "../App";

interface Props {
  item: Item;
  onExit: () => void;
  onToast: (text: string, tone?: Toast["tone"]) => void;
}

/** 阅读器：竖向拼接图片上下滑动；点击画面显隐浮窗工具栏 */
export default function Reader({ item, onExit, onToast }: Props) {
  const [pages, setPages] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadedCount, setLoadedCount] = useState(0);
  const [chromeVisible, setChromeVisible] = useState(true);
  const [current, setCurrent] = useState(1);
  const [fraction, setFraction] = useState(item.manga?.progress ?? 0);

  const scrollRef = useRef<HTMLDivElement>(null);
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

  const jumpTo = (f: number) => {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTo({ top: f * (el.scrollHeight - el.clientHeight) });
  };

  return (
    <div className="reader">
      <div
        className={`reader-topbar${chromeVisible ? "" : " chrome-hidden"}`}
        onClick={(e) => e.stopPropagation()}
      >
        <button type="button" className="btn" onClick={onExit}>
          ← 退出阅读
        </button>
        <span className="reader-title">{item.title}</span>
        <span className="reader-counter">
          {pages.length > 0 ? `${current} / ${pages.length}` : "—"}
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
          <div className="reader-track">
            {pages.map((p, i) => (
              <img
                key={p}
                className="reader-page"
                src={convertFileSrc(p)}
                alt={`第 ${i + 1} 页`}
                draggable={false}
                onLoad={() => setLoadedCount((c) => c + 1)}
                onError={() => setLoadedCount((c) => c + 1)}
              />
            ))}
          </div>
        )}
      </div>

      <div
        className={`reader-bottombar${chromeVisible ? "" : " chrome-hidden"}`}
        onClick={(e) => e.stopPropagation()}
      >
        <input
          className="reader-slider"
          type="range"
          min={0}
          max={1000}
          value={Math.round(fraction * 1000)}
          onChange={(e) => jumpTo(Number(e.target.value) / 1000)}
          aria-label="阅读进度"
        />
        <span className="reader-counter">{Math.round(fraction * 100)}%</span>
        <button
          type="button"
          className="btn"
          onClick={() => jumpTo(0)}
        >
          回到顶部
        </button>
        <button type="button" className="btn btn-primary" onClick={onExit}>
          退出阅读
        </button>
      </div>

      {chromeVisible && pages.length > 0 && (
        <div className="reader-hint">
          点击画面显隐工具栏 · 滚轮/方向键翻页 · Esc 退出
        </div>
      )}
    </div>
  );
}
