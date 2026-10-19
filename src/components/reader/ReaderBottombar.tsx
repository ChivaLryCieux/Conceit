import { MinusIcon, PlusIcon } from "../ui/icons";

interface Props {
  title: string;
  /** 缩放比例，1 = 宽度填充屏幕 */
  zoom: number;
  /** 是否已到缩放边界（用于禁用按钮） */
  zoomAtMin: boolean;
  zoomAtMax: boolean;
  onZoomIn: () => void;
  onZoomOut: () => void;
  onZoomReset: () => void;
  /** 当前阅读进度 0-1 */
  fraction: number;
  current: number;
  total: number;
  /** 工具栏是否可见（隐藏时仅淡出，不影响漫画内容） */
  chromeVisible: boolean;
  /** 拖动进度滑块：跳转到指定进度 */
  onSeek: (fraction: number) => void;
  onExit: () => void;
}

/** 阅读器唯一底栏：退出、标题、缩放、进度滑块与计数。
 *  放在底部而非顶部——顶部紧邻手机状态栏，容易误触。 */
export default function ReaderBottombar({
  title,
  zoom,
  zoomAtMin,
  zoomAtMax,
  onZoomIn,
  onZoomOut,
  onZoomReset,
  fraction,
  current,
  total,
  chromeVisible,
  onSeek,
  onExit,
}: Props) {
  return (
    <div
      className={`reader-bottombar${chromeVisible ? "" : " chrome-hidden"}`}
      onClick={(e) => e.stopPropagation()}
    >
      <button type="button" className="btn" onClick={onExit}>
        ← 退出阅读
      </button>
      <span className="reader-title">{title}</span>

      <div className="reader-zoom" onClick={(e) => e.stopPropagation()}>
        <button
          type="button"
          className="btn btn-icon"
          title="缩小"
          aria-label="缩小"
          disabled={zoomAtMin}
          onClick={onZoomOut}
        >
          <MinusIcon />
        </button>
        <button
          type="button"
          className="reader-zoom-value"
          title="恢复适应宽度"
          onClick={onZoomReset}
        >
          {Math.round(zoom * 100)}%
        </button>
        <button
          type="button"
          className="btn btn-icon"
          title="放大"
          aria-label="放大"
          disabled={zoomAtMax}
          onClick={onZoomIn}
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
        onChange={(e) => onSeek(Number(e.target.value) / 1000)}
        onClick={(e) => e.stopPropagation()}
        aria-label="阅读进度"
      />
      <span className="reader-counter">
        {total > 0
          ? `${current} / ${total} · ${Math.round(fraction * 100)}%`
          : "—"}
      </span>
    </div>
  );
}
