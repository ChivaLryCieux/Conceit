import { convertFileSrc } from "@tauri-apps/api/core";
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
  onContextMenu: (e: React.MouseEvent) => void;
  onDragStart: (e: React.DragEvent) => void;
  onDragEnd: () => void;
  onDrop: (e: React.DragEvent) => void;
}

export default function Card({
  item,
  root,
  mangaCount,
  indicator,
  dragging,
  onActivate,
  onContextMenu,
  onDragStart,
  onDragEnd,
  onDrop,
}: Props) {
  const isFolder = item.kind === "folder";
  const coverUrl =
    item.manga?.cover != null
      ? convertFileSrc(`${root}/${item.manga.dir}/${item.manga.cover}`)
      : null;
  const progress = item.manga?.progress ?? 0;
  const pageCount = item.manga?.pages.length ?? 0;

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
      <div className="card-cover">
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
          <div className="card-cover-empty">
            <span>🖼️</span>
            <span className="card-cover-empty-text">暂无图片</span>
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
        {item.title}
        {!isFolder && pageCount > 0 && (
          <span className="card-pages">{pageCount} 页</span>
        )}
      </div>
    </div>
  );
}
