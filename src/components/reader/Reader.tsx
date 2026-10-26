import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { fetchPages, saveProgress } from "../../api";
import { useBackHandler } from "../../backHandler";
import ReaderBottombar from "./ReaderBottombar";
import { useReaderWindow } from "./useReaderWindow";
import type { Item, Toast } from "../../types";

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

/** 阅读器：竖向拼接图片上下滑动；点击画面显隐唯一底部框栏。
 *  图片为窗口化懒加载：只有视口附近的几张挂 <img>，其余用等高占位
 *  撑高度（见 useReaderWindow），数百页的漫画也不会一次性解码全部原图。 */
export default function Reader({ item, onExit, onToast }: Props) {
  const [pages, setPages] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);
  const [chromeVisible, setChromeVisible] = useState(true);
  /** 缩放比例：1 = 宽度填充整个屏幕 */
  const [zoom, setZoom] = useState(1);

  const scrollRef = useRef<HTMLDivElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);

  /* ---------- 系统返回（安卓硬件返回键 / 边缘滑动手势）→ 退出阅读 ---------- */
  useBackHandler(() => {
    onExit();
    return true;
  });

  /* ---------- 加载页列表 ---------- */
  useEffect(() => {
    let alive = true;
    setLoading(true);
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

  /* ---------- 每页宽高比（索引里的尺寸元数据；旧数据没有则回退估算） ---------- */
  const aspects = useMemo(() => {
    const dims = item.manga?.dims;
    return pages.map((p) => {
      const d = dims?.[p];
      return d && d[0] > 0 && d[1] > 0 ? d[1] / d[0] : undefined;
    });
  }, [pages, item.manga?.dims]);

  /* ---------- 窗口化懒加载：渲染范围 / 当前页 / 进度 / 缩放换算 ---------- */
  const win = useReaderWindow({
    scrollRef,
    pageCount: pages.length,
    aspects,
    zoom,
    progress: item.manga?.progress ?? 0,
    onSaveProgress: useCallback((f: number) => {
      void saveProgress(item.id, f).catch(() => {});
    }, [item.id]),
  });

  /* ---------- 退出时保存进度 ---------- */
  useEffect(() => {
    const fractionRef = win.fractionRef;
    return () => {
      void saveProgress(item.id, fractionRef.current).catch(() => {});
    };
  }, [item.id, win.fractionRef]);

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

  const zoomed = zoom > 1.001;
  const { start, end } = win.range;

  return (
    <div className="reader" ref={rootRef}>
      <ReaderBottombar
        title={item.title}
        zoom={zoom}
        zoomAtMin={zoom <= ZOOM_MIN}
        zoomAtMax={zoom >= ZOOM_MAX}
        onZoomIn={() => setZoom((z) => clampZoom(z + 0.1))}
        onZoomOut={() => setZoom((z) => clampZoom(z - 0.1))}
        onZoomReset={() => setZoom(1)}
        fraction={win.fraction}
        current={win.current}
        total={pages.length}
        chromeVisible={chromeVisible}
        onSeek={win.jumpTo}
        onExit={onExit}
      />

      <div
        className="reader-scroll"
        ref={scrollRef}
        onScroll={win.onScroll}
        onClick={() => setChromeVisible((v) => !v)}
      >
        {loading ? (
          <div className="reader-loading">正在加载图片…</div>
        ) : pages.length === 0 ? (
          <div className="reader-loading">这部漫画还没有图片</div>
        ) : (
          <div className={`reader-track${zoomed ? " reader-track-zoomed" : ""}`}>
            {/* 窗口之外的页不渲染 <img>，用等高占位撑出正确滚动高度 */}
            {win.spacerTop > 0 && (
              <div className="reader-spacer" style={{ height: win.spacerTop }} />
            )}
            {pages.slice(start, end).map((p, i) => {
              const index = start + i;
              return (
                <div
                  className="reader-page-slot"
                  key={p}
                  style={{ height: win.heights[index] || win.estimateAt(index) }}
                >
                  <img
                    className="reader-page"
                    src={convertFileSrc(p)}
                    alt={`第 ${index + 1} 页`}
                    style={{ width: `${zoom * 100}%` }}
                    draggable={false}
                    onLoad={(e) =>
                      win.measure(index, e.currentTarget.clientHeight)
                    }
                    onError={() => win.measure(index, win.estimateAt(index))}
                  />
                </div>
              );
            })}
            {win.spacerBottom > 0 && (
              <div
                className="reader-spacer"
                style={{ height: win.spacerBottom }}
              />
            )}
          </div>
        )}
      </div>
    </div>
  );
}
