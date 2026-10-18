import { useEffect, useRef, useState } from "react";

export interface MenuAction {
  label: string;
  onSelect: () => void;
  danger?: boolean;
  separatorBefore?: boolean;
}

interface Props {
  x: number;
  y: number;
  actions: MenuAction[];
  onClose: () => void;
}

/** 轻量右键/长按菜单：点击外部 / Esc / 滚动 / 缩放时关闭 */
export default function ContextMenu({ x, y, actions, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  /** 先用估算值定位，挂载后按真实尺寸夹取，避免小屏上溢出屏幕 */
  const [pos, setPos] = useState(() => ({
    left: Math.max(8, Math.min(x, window.innerWidth - 190)),
    top: Math.max(8, Math.min(y, window.innerHeight - actions.length * 36 - 24)),
  }));

  useEffect(() => {
    const close = () => onClose();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("click", close);
    window.addEventListener("contextmenu", close);
    window.addEventListener("keydown", onKey);
    window.addEventListener("resize", close);
    window.addEventListener("scroll", close, true);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("contextmenu", close);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", close);
      window.removeEventListener("scroll", close, true);
    };
  }, [onClose]);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({
      left: Math.max(8, Math.min(x, window.innerWidth - r.width - 8)),
      top: Math.max(8, Math.min(y, window.innerHeight - r.height - 8)),
    });
  }, [x, y]);

  return (
    <div
      ref={ref}
      className="ctx-menu"
      style={{ left: pos.left, top: pos.top }}
      onClick={(e) => e.stopPropagation()}
      onContextMenu={(e) => e.preventDefault()}
    >
      {actions.map((a) => (
        <div key={a.label}>
          {a.separatorBefore && <div className="ctx-sep" />}
          <button
            type="button"
            className={`ctx-item${a.danger ? " ctx-danger" : ""}`}
            onClick={() => {
              a.onSelect();
              onClose();
            }}
          >
            {a.label}
          </button>
        </div>
      ))}
    </div>
  );
}
