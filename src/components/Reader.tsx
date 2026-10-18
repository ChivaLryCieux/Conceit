import { useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { fetchPages } from "../api";
import type { Item } from "../types";
import type { Toast } from "../App";

interface Props {
  item: Item;
  onExit: () => void;
  onToast: (text: string, tone?: Toast["tone"]) => void;
}

/** 阅读器：竖向拼接图片、上下滑动；点击呼出/隐藏浮窗工具栏 */
export default function Reader({ item, onExit, onToast }: Props) {
  const [pages, setPages] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);
  const [chromeVisible, setChromeVisible] = useState(true);
  const [current, setCurrent] = useState(1);
  const scrollRef = useRef<HTMLDivElement>(null);

  // 加载图片列表
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

  // Esc 退出
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onExit();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onExit]);

  // 滚动时更新当前页码
  useEffect(() => {
    const scroller = scrollRef.current;
    if (!scroller) return;
    const onScroll = () => {
      const mid = scroller.scrollTop + scroller.clientHeight / 2;
      const imgs = Array.from(
        scroller.querySelectorAll<HTMLImageElement>(".reader-page")
      );
      let idx = 0;
      for (let i = 0; i < imgs.length; i += 1) {
        if (imgs[i].offsetTop <= mid) idx = i;
      }
      setCurrent(Math.min(idx + 1, Math.max(pages.length, 1)));
    };
    scroller.addEventListener("scroll", onScroll, { passive: true });
    return () => scroller.removeEventListener("scroll", onScroll);
  }, [pages.length]);

  return (
    <div className="reader">
      <div className={`reader-topbar${chromeVisible ? "" : " chrome-hidden"}`}>
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
                loading={i < 3 ? "eager" : "lazy"}
                draggable={false}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
