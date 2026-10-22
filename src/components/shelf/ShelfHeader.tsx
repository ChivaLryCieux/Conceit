import type { DragEvent } from "react";
import { BookLogo } from "../ui/icons";
import type { Crumb } from "./useShelfMutations";

interface Props {
  breadcrumbs: Crumb[];
  /** 面包屑上的拖放落点（索引） */
  crumbOver: number | null;
  onNavigateDepth: (depth: number) => void;
  onImportDir: () => void;
  onImportZip: () => void;
  onCreateFolder: () => void;
  /** 新建一部空漫画（之后在它的设置里手动添加图片） */
  onCreateManga: () => void;
  /** 打开书库体检（索引与磁盘实况对比） */
  onLibraryHealth: () => void;
  onCrumbDragOver: (e: DragEvent, index: number) => void;
  onCrumbDragLeave: () => void;
  onCrumbDrop: (e: DragEvent, folderId: string | null) => void;
}

/** 书架头部：品牌区、面包屑导航与操作按钮 */
export default function ShelfHeader({
  breadcrumbs,
  crumbOver,
  onNavigateDepth,
  onImportDir,
  onImportZip,
  onCreateFolder,
  onCreateManga,
  onLibraryHealth,
  onCrumbDragOver,
  onCrumbDragLeave,
  onCrumbDrop,
}: Props) {
  return (
    <header className="shelf-header">
      <div className="shelf-brand">
        <span className="brand-mark">
          <BookLogo />
        </span>
        <span className="brand-name">幻想阅读器</span>
        <span className="brand-sub">Conceit</span>
      </div>
      <nav className="crumbs">
        {breadcrumbs.map((c, i) => (
          <span key={c.id ?? "root"} className="crumb-wrap">
            {i > 0 && <span className="crumb-sep">/</span>}
            <button
              type="button"
              className={`crumb${
                i === breadcrumbs.length - 1 ? " crumb-current" : ""
              }${crumbOver === i ? " crumb-droppable" : ""}`}
              onClick={() => onNavigateDepth(i)}
              onDragOver={(e) => onCrumbDragOver(e, i)}
              onDragLeave={onCrumbDragLeave}
              onDrop={(e) => void onCrumbDrop(e, c.id)}
            >
              {c.title}
            </button>
          </span>
        ))}
      </nav>
      <div className="shelf-actions">
        <button type="button" className="btn" onClick={onImportDir}>
          ＋ 导入文件夹
        </button>
        <button type="button" className="btn" onClick={onImportZip}>
          ＋ 导入 zip
        </button>
        <button
          type="button"
          className="btn btn-primary"
          onClick={onCreateFolder}
        >
          新建文件夹
        </button>
        <button type="button" className="btn" onClick={onCreateManga}>
          新建漫画
        </button>
        <button type="button" className="btn" onClick={onLibraryHealth}>
          书库体检
        </button>
      </div>
    </header>
  );
}
