import { convertFileSrc } from "@tauri-apps/api/core";
import { GearIcon } from "./icons";
import type { Item } from "../types";

export type DropMode = "before" | "after" | "into";

export interface DragIndicator {
  mode: DropMode | null;
}

interface Props {
  item: Item;
  /** 书库根目录（绝对路径，/ 分隔） */
  root: string;
  /** 文件夹卡片显示其中的漫画数量 */
  mangaCount?: number;
  indicator: DragIndicator;
  dragging: boolean;
  onActivate: () => void;
  onSettings: () => void;
  onContextMenu: (e: React.MouseEvent) => void;
  onDragStart: (e: React.DragEvent) => void;
  onDragEnd: () => void;
  onDrop: (e: React.DragEvent) => void;
}

/** 由 id 派生一个稳定的纯色（HSL） */
function solidColor(id: string): { bg: string; fg: string } {
  let h = 0;
  for (let i = 0; i < id.length; i += 1) {
    h = (h * 31 + id.charCodeAt(i)) % 360;
  }
  return {
    bg: `hsl(${h}, 30%, 27%)`,
    fg: `hsl(${h}, 42%, 66%)`,
  };
}

export default function Card({
  item,
  root,
  mangaCount,
  indicator,
  dragging,
  onActivate,
  onSettings,
  onContextMenu,
  onDragStart,
  onDragEnd,
  onDrop,
}: Props) {
  const isFolder = item.kind === "folder";
  const coverPath = item.manga?.cover;
  const showImage = !isFolder && item.showCover && coverPath != null;
  const coverUrl = showImage
    ? convertFileSrc(`${root}/${item.manga!.dir}/${coverPath}`)
    : null;
  const progress = item.manga?.progress ?? 0;
  const pageCount = item.manga?.pages.length ?? 0;
  const solid = solidColor(item.id);
  const initial = item.title.trim().charAt(0) || "·";

  const classes = [
    "card",
    isFolder ? "card-folder" : "card-manga",
    dragging ? "card-dragging" : "",
    indicator.mode ? `card-drop-${indicator.mode}` : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div
      data-card-id={item.id}
      className={classes}
      draggable
      onClick={onActivate}
      onContextMenu={(e) => {
        e.preventDefault();
        onContextMenu(e);
      }}
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      onDrop={onDrop}
      title={item.title}
    >
      <div
        className="card-cover"
        style={coverUrl ? undefined : { background: solid.bg }}
      >
        {isFolder ? (
          <div className="card-cover-folder">
            <span className="folder-icon">📁</span>
            {mangaCount != null && (
              <span className="folder-count">{mangaCount} 部漫画</span>
            )}
          </div>
        ) : coverUrl ? (
          <img src={coverUrl} alt={item.title} loading="lazy" draggable={false} />
        ) : (
          <div className="card-cover-solid">
            <span className="card-cover-initial" style={{ color: solid.fg }}>
              {initial}
            </span>
          </div>
        )}

        {!isFolder && progress > 0.01 && (
          <>
            <div className="card-progress">
              <div
                className="card-progress-fill"
                style={{ width: `${Math.round(progress * 100)}%` }}
              />
            </div>
            <span className="card-progress-label">
              {Math.round(progress * 100)}%
            </span>
          </>
        )}
      </div>
      <div className="card-title">
        <span className="card-title-text">{item.title}</span>
        <button
          type="button"
          className="card-settings"
          title="设置"
          aria-label={`设置 ${item.title}`}
          onClick={(e) => {
            e.stopPropagation();
            onSettings();
          }}
          onMouseDown={(e) => e.preventDefault()}
        >
          <GearIcon />
        </button>
      </div>
      {!isFolder && pageCount > 0 && (
        <div className="card-meta">{pageCount} 页</div>
      )}
    </div>
  );
}
