import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useBackHandler } from "../../backHandler";
import { CheckIcon } from "./icons";

export interface MenuAction {
  label: string;
  onSelect: () => void;
  danger?: boolean;
  separatorBefore?: boolean;
  /** 勾选位（风格切换这类单选项用）；undefined = 不显示勾选位 */
  checked?: boolean;
}

interface Props {
  x: number;
  y: number;
  actions: MenuAction[];
  onClose: () => void;
}

/** 轻量右键/长按菜单：点击外部 / Esc / 滚动 / 缩放时关闭；安卓系统返回同样关闭 */
export default function ContextMenu({ x, y, actions, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  /** 先用估算值定位，挂载后按真实尺寸夹取，避免小屏上溢出屏幕 */
  const [pos, setPos] = useState(() => ({
    left: Math.max(8, Math.min(x, window.innerWidth - 190)),
    top: Math.max(8, Math.min(y, window.innerHeight - actions.length * 36 - 24)),
  }));

  // 系统返回（安卓硬件返回键 / 边缘滑动手势）关闭菜单
  useBackHandler(() => {
    onClose();
    return true;
  });

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
    // 打开另一个菜单前主动广播：打开方会拦下自己的 click 冒泡，
    // 已开着的菜单听不到 click，靠这个事件收起
    window.addEventListener("conceit:close-menus", close);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("contextmenu", close);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", close);
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("conceit:close-menus", close);
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

  /* Portal 到 body：玻璃胶囊（backdrop-filter）会成为 fixed 后代的包含块，
     留在 header 里菜单的坐标会变成相对胶囊、被推出屏幕外。
     transformOrigin 按锚点方位设置，让菜单从触发处缩放弹出 */
  const originY = y > window.innerHeight / 2 ? "bottom" : "top";
  const originX = x > window.innerWidth / 2 ? "right" : "left";

  return createPortal(
    <div
      ref={ref}
      className="ctx-menu glass"
      style={{ left: pos.left, top: pos.top, transformOrigin: `${originY} ${originX}` }}
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
            {a.checked !== undefined && (
              <span className="ctx-check" aria-hidden="true">
                {a.checked ? <CheckIcon /> : null}
              </span>
            )}
            <span className="ctx-item-label">{a.label}</span>
          </button>
        </div>
      ))}
    </div>,
    document.body
  );
}
