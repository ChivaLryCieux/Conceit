import { useState, type DragEvent, type MouseEvent } from "react";
import ContextMenu, { type MenuAction } from "../ui/ContextMenu";
import { GearIcon, PlusIcon } from "../ui/icons";
import { useTheme } from "../../theme";
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

/** 底部功能栏：悬浮玻璃胶囊（覆盖在网格上方，封面从它下面滑过）。
 *  左下角品牌（纯文本，不带框）、面包屑导航、右下角两个按钮——
 *  新建（右起第二个）与设置（最右）。两个按钮都在自己上方弹出菜单：
 *  新建 = 新建漫画 / 新建文件夹 / 导入 zip / 导入文件夹；
 *  设置 = 书库体检 / 界面风格（白色 / 黑色）。 */
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
  const [theme, setTheme] = useTheme();
  const [menu, setMenu] = useState<{
    key: "new" | "settings";
    x: number;
    y: number;
    actions: MenuAction[];
  } | null>(null);

  /** 在触发按钮的正上方、右对齐弹出菜单 */
  function openAbove(
    e: MouseEvent<HTMLButtonElement>,
    key: "new" | "settings",
    actions: MenuAction[]
  ) {
    // 拦下这次 click 的冒泡：已开着的菜单监听 window click 收起自己，
    // 不拦的话「点另一个按钮换菜单」会被先关掉。同时广播一个收起事件，
    // 让本组件之外还开着的菜单（如卡片右键菜单）也收起。
    e.stopPropagation();
    window.dispatchEvent(new CustomEvent("conceit:close-menus"));
    const r = e.currentTarget.getBoundingClientRect();
    setMenu({
      key,
      x: r.right - 180,
      y: r.top - actions.length * 42 - 14,
      actions,
    });
  }

  const newActions: MenuAction[] = [
    { label: "新建漫画", onSelect: onCreateManga },
    { label: "新建文件夹", onSelect: onCreateFolder },
    { label: "导入 zip", onSelect: onImportZip, separatorBefore: true },
    { label: "导入文件夹", onSelect: onImportDir },
  ];

  const settingsActions: MenuAction[] = [
    { label: "书库体检", onSelect: onLibraryHealth },
    {
      label: "界面风格：白色",
      checked: theme === "white",
      onSelect: () => setTheme("white"),
      separatorBefore: true,
    },
    {
      label: "界面风格：黑色",
      checked: theme === "black",
      onSelect: () => setTheme("black"),
    },
  ];

  return (
    <header className="shelf-header glass">
      <div className="shelf-brand">
        <span className="brand-name">Conceit 幻想阅读器</span>
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
        <button
          type="button"
          className="btn"
          title="新建 / 导入"
          aria-label="新建 / 导入"
          onClick={(e) => openAbove(e, "new", newActions)}
        >
          <PlusIcon size={17} />
        </button>
        <button
          type="button"
          className="btn"
          title="设置"
          aria-label="设置"
          onClick={(e) => openAbove(e, "settings", settingsActions)}
        >
          <GearIcon size={17} />
        </button>
      </div>
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          actions={menu.actions}
          onClose={() => setMenu(null)}
        />
      )}
    </header>
  );
}
