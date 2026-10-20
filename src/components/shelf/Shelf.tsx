import { useMemo, useRef, useState } from "react";
import Card from "./Card";
import ContextMenu, { type MenuAction } from "../ui/ContextMenu";
import InputDialog from "../ui/InputDialog";
import ItemSettings from "../ItemSettings";
import ShelfHeader from "./ShelfHeader";
import { childrenOf } from "../../api";
import type { Item, LibrarySnapshot, Toast } from "../../types";
import { useItemSorting } from "./useItemSorting";
import { useShelfMutations, type Crumb } from "./useShelfMutations";

interface Props {
  snapshot: LibrarySnapshot;
  currentFolder: string | null;
  breadcrumbs: Crumb[];
  onEnterFolder: (id: string) => void;
  onNavigateDepth: (depth: number) => void;
  onOpenManga: (item: Item) => void;
  onReload: () => Promise<void> | void;
  onToast: (text: string, tone?: Toast["tone"]) => void;
  onImport: (paths: string[]) => Promise<Item[]>;
}

/** 书架主页：底部功能栏 + 面包屑 + 卡片网格，负责把各模块组合起来 */
export default function Shelf({
  snapshot,
  currentFolder,
  breadcrumbs,
  onEnterFolder,
  onNavigateDepth,
  onOpenManga,
  onReload,
  onToast,
  onImport,
}: Props) {
  /** 操作菜单（右键 / 长按） */
  const [menu, setMenu] = useState<{ x: number; y: number; item: Item } | null>(
    null
  );
  /** 新建文件夹弹窗 */
  const [folderDialog, setFolderDialog] = useState(false);
  /** 条目设置弹窗 */
  const [settingsItem, setSettingsItem] = useState<Item | null>(null);
  const gridRef = useRef<HTMLDivElement>(null);

  // 书架条目只在快照或所在目录变化时重算（排序过程中会频繁触发重渲染）
  const items = useMemo(
    () => childrenOf(snapshot.index, currentFolder),
    [snapshot.index, currentFolder]
  );

  const { busy, pickAndImport, createFolderByName, removeItem, moveTo } =
    useShelfMutations({
      currentFolder,
      breadcrumbs,
      onImport,
      onReload,
      onToast,
      onNavigateDepth,
    });

  const { dragId, over, crumbOver, handlers } = useItemSorting({
    items,
    allItems: snapshot.index.items,
    currentFolder,
    gridRef,
    onMoveTo: moveTo,
    onLongPress: (x, y, item) => setMenu({ x, y, item }),
    onReload,
    onToast,
  });

  function activate(item: Item) {
    if (item.kind === "folder") onEnterFolder(item.id);
    else onOpenManga(item);
  }

  function menuActions(item: Item): MenuAction[] {
    const actions: MenuAction[] = [
      {
        label: item.kind === "folder" ? "打开文件夹" : "开始阅读",
        onSelect: () => activate(item),
      },
      { label: "重命名", onSelect: () => setSettingsItem(item) },
    ];
    if (item.parent) {
      actions.push({
        label: "移动到根目录",
        onSelect: () => void moveTo(item, null),
      });
    }
    actions.push({
      label: "删除",
      danger: true,
      separatorBefore: true,
      onSelect: () => void removeItem(item),
    });
    return actions;
  }

  return (
    <div className="shelf">
      {items.length === 0 ? (
        <div className="shelf-empty">
          <p className="shelf-empty-title">这个书架还空着</p>
          <p>
            点击「导入文件夹」选择本机漫画目录，或把文件夹 / zip
            直接拖到窗口里
          </p>
          <button
            type="button"
            className="btn btn-primary"
            onClick={() => void pickAndImport("dir")}
          >
            导入漫画
          </button>
        </div>
      ) : (
        <div
          className="shelf-grid"
          ref={gridRef}
          onDragOver={handlers.onGridDragOver}
          onDrop={(e) => void handlers.onGridDrop(e)}
        >
          {items.map((item) => (
            <Card
              key={item.id}
              item={item}
              root={snapshot.root}
              mangaCount={
                item.kind === "folder"
                  ? (snapshot.manga_counts[item.id] ?? 0)
                  : undefined
              }
              indicator={
                over.id === item.id && over.id !== dragId
                  ? { mode: over.mode }
                  : { mode: null }
              }
              dragging={dragId === item.id}
              onActivate={() => activate(item)}
              onSettings={() => setSettingsItem(item)}
              onContextMenu={(e) => setMenu({ x: e.clientX, y: e.clientY, item })}
              onDragStart={(e) => handlers.onCardDragStart(e, item)}
              onDragEnd={handlers.onCardDragEnd}
              onDrop={handlers.onCardDrop}
            />
          ))}
          {dragId && over.id === null && <div className="drop-endline" />}
        </div>
      )}

      {/* 功能栏放底部：平板 / 手机上拇指更好够到，顶部也不再被状态栏占掉一块 */}
      <ShelfHeader
        breadcrumbs={breadcrumbs}
        crumbOver={crumbOver}
        onNavigateDepth={onNavigateDepth}
        onImportDir={() => void pickAndImport("dir")}
        onImportZip={() => void pickAndImport("zip")}
        onCreateFolder={() => setFolderDialog(true)}
        onCrumbDragOver={handlers.onCrumbDragOver}
        onCrumbDragLeave={handlers.onCrumbDragLeave}
        onCrumbDrop={handlers.onCrumbDrop}
      />

      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          actions={menuActions(menu.item)}
          onClose={() => setMenu(null)}
        />
      )}

      {folderDialog && (
        <InputDialog
          title="新建文件夹"
          label="文件夹名称"
          placeholder="例如：本周更新"
          confirmText="创建"
          onConfirm={async (name) => {
            if (await createFolderByName(name)) setFolderDialog(false);
          }}
          onClose={() => setFolderDialog(false)}
        />
      )}

      {settingsItem && (
        <ItemSettings
          item={settingsItem}
          root={snapshot.root}
          onClose={() => setSettingsItem(null)}
          onSaved={() => onReload()}
          onToast={onToast}
        />
      )}

      {busy && (
        <div className="busy-mask">
          <div className="spinner" />
          <div>{busy}</div>
        </div>
      )}
    </div>
  );
}
