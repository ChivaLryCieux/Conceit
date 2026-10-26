import { useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import Modal from "./ui/Modal";
import {
  addMangaPages,
  clearImportCache,
  deleteMangaPages,
  exportManga,
  isAndroid,
  pickImages,
  reorderMangaPages,
  updateItemSettings,
  type ExportProgress,
} from "../api";
import { Channel } from "@tauri-apps/api/core";
import { usePageSorting } from "../usePageSorting";
import type { Item } from "../types";

interface Props {
  item: Item;
  /** 书库根目录（绝对路径） */
  root: string;
  onClose: () => void;
  onSaved: () => void | Promise<void>;
  onToast: (text: string, tone?: "info" | "error" | "success") => void;
  /** 请求删除该条目（二次确认与执行由 Shelf 负责） */
  onDelete: (item: Item) => void;
}

/** 条目设置：名称、是否展示封面、选择漫画某一页作为封面；
 *  漫画 additionally 支持在这里增加 / 删除图片与拖动排序。 */
export default function ItemSettings({
  item,
  root,
  onClose,
  onSaved,
  onToast,
  onDelete,
}: Props) {
  const [title, setTitle] = useState(item.title);
  const [showCover, setShowCover] = useState(item.show_cover);
  const [cover, setCover] = useState<string | undefined>(item.manga?.cover);
  const [pages, setPages] = useState<string[]>(item.manga?.pages ?? []);
  /** 标记待删除的页面（点 ✕ 标记，统一确认后删除，防误触） */
  const [pendingDelete, setPendingDelete] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  /** 导出进度（ratio 为 null = 总量未知，走不确定态）；null 表示没在导出 */
  const [exporting, setExporting] = useState<{
    ratio: number | null;
    label: string;
  } | null>(null);
  const pickerRef = useRef<HTMLDivElement>(null);

  const isManga = item.kind === "manga";
  const mangaDir = item.manga?.dir ?? "";
  const pageSrc = (p: string) => convertFileSrc(`${root}/${mangaDir}/${p}`);

  /** 增删 / 排序后：用后端返回的最新条目同步本地状态，并通知书架刷新 */
  function applyMutation(updated: Item) {
    const nextPages = updated.manga?.pages ?? [];
    setPages(nextPages);
    // 用户可能已手动选过封面（未保存）：只要那页还在就保留，否则跟随后端
    setCover((prev) =>
      prev && nextPages.includes(prev) ? prev : updated.manga?.cover
    );
    setPendingDelete(new Set());
    void onSaved();
  }

  /** 拖动排序：先本地生效，失败回滚 */
  async function persistReorder(next: string[]) {
    const prev = pages;
    setPages(next);
    try {
      applyMutation(await reorderMangaPages(item.id, next));
    } catch (e) {
      setPages(prev);
      onToast(`保存顺序失败：${e}`, "error");
    }
  }

  const sorting = usePageSorting(pickerRef, pages, persistReorder);

  const save = async () => {
    if (!title.trim()) return;
    setBusy(true);
    try {
      await updateItemSettings(item.id, {
        title: title.trim(),
        showCover,
        cover: isManga ? cover : undefined,
      });
      await onSaved();
      onToast("设置已保存", "success");
      onClose();
    } catch (e) {
      onToast(`保存失败：${e}`, "error");
    } finally {
      setBusy(false);
    }
  };

  /** 添加图片：桌面走插件对话框，安卓走原生桥多选（返回缓存真实路径） */
  async function addImages() {
    setBusy(true);
    try {
      const android = await isAndroid();
      let paths: string[] = [];
      if (android) {
        paths = await pickImages();
      } else {
        const { open } = await import("@tauri-apps/plugin-dialog");
        const selected = await open({
          multiple: true,
          title: "选择要添加的图片",
          filters: [
            {
              name: "图片",
              extensions: ["jpg", "jpeg", "png", "webp", "gif", "bmp", "avif"],
            },
          ],
        });
        if (selected) paths = Array.isArray(selected) ? selected : [selected];
      }
      if (paths.length === 0) return;
      const before = pages.length;
      const updated = await addMangaPages(item.id, paths);
      const added = (updated.manga?.pages.length ?? 0) - before;
      applyMutation(updated);
      onToast(
        added > 0 ? `已添加 ${added} 页（排在末尾，可拖动归位）` : "没有新增页面",
        added > 0 ? "success" : "info"
      );
      if (android) {
        // 书库已有自己的副本，清掉原生桥复制进缓存的临时文件
        await clearImportCache().catch(() => {});
      }
    } catch (e) {
      onToast(`添加图片失败：${e}`, "error");
    } finally {
      setBusy(false);
    }
  }

  /** 删除标记的页面（文件与索引一并删除） */
  async function removePending() {
    const targets = [...pendingDelete];
    if (targets.length === 0) return;
    const ok = window.confirm(
      `确定删除所选 ${targets.length} 页吗？文件会从书库中移除，无法恢复。`
    );
    if (!ok) return;
    setBusy(true);
    try {
      applyMutation(await deleteMangaPages(item.id, targets));
      onToast(`已删除 ${targets.length} 页`, "success");
    } catch (e) {
      onToast(`删除失败：${e}`, "error");
    } finally {
      setBusy(false);
    }
  }

  /** 有任一异步操作在跑（增删页 / 保存 / 导出）时禁用其他按钮，
   *  避免两个命令同时改同一部漫画 */
  const working = busy || exporting !== null;

  /** 导出成 zip 落到系统下载目录：进度经 Channel 回报，弹窗内显示进度条。
   *  桌面端直接写 download_dir()；安卓上由原生桥写进公共下载目录
   *  （Android 10+ 走 MediaStore，无需权限；更低版本弹 SAF 保存选择器）。 */
  async function exportZip() {
    if (pages.length === 0) return;
    setExporting({ ratio: null, label: "正在准备…" });
    const channel = new Channel<ExportProgress>((msg) => {
      const ratio =
        msg.totalBytes > 0
          ? Math.min(1, Math.max(0, msg.doneBytes / msg.totalBytes))
          : null;
      const pct = ratio === null ? "" : ` ${Math.round(ratio * 100)}%`;
      setExporting({
        ratio,
        label:
          msg.phase === "save"
            ? `正在保存到下载目录…${pct}`
            : `正在打包…${pct}`,
      });
    });
    try {
      const dest = await exportManga(item.id, channel);
      onToast(`已导出到 ${dest}`, "success");
    } catch (e) {
      // 安卓 9 及以下走 SAF 选择器，用户按返回键取消不是出错
      const cancelled = String(e) === "已取消导出";
      onToast(cancelled ? "已取消导出" : `导出失败：${e}`, cancelled ? "info" : "error");
    } finally {
      setExporting(null);
    }
  }

  function togglePending(page: string) {
    setPendingDelete((prev) => {
      const next = new Set(prev);
      if (next.has(page)) next.delete(page);
      else next.add(page);
      return next;
    });
  }

  /** 危险操作区：只有一个删除入口，文案刻意保持最短——「删掉什么、后果多严重」
   *  由随后的二次确认框（Shelf 的 ConfirmDialog）讲清楚，这里不再重复解释。
   *  放在弹窗最底部：读者滚到这儿已有心理准备，也不会在缩略图网格里误触。 */
  const dangerZone = (
    <div className="settings-danger">
      <button
        type="button"
        className="btn btn-danger"
        onClick={() => onDelete(item)}
      >
        {isManga ? "删除漫画" : "删除文件夹"}
      </button>
    </div>
  );

  /** 导出进行中不关弹窗：进度 Channel 还连着，中途关掉用户既看不到结果，
   *  也不知道该不该重来。导出本身是有界的（打包 + 复制），不会卡住。 */
  function requestClose() {
    if (exporting) return;
    onClose();
  }

  return (
    <Modal
      title={`设置 · ${item.title}`}
      onClose={requestClose}
      wide={isManga && pages.length > 0}
      footer={
        <>
          <button type="button" className="btn" onClick={requestClose}>
            取消
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={working || !title.trim()}
            onClick={() => void save()}
          >
            {working ? "处理中…" : "保存"}
          </button>
        </>
      }
    >
      <label className="field">
        <span className="field-label">名称</span>
        <input
          className="input"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void save();
          }}
        />
      </label>

      {isManga ? (
        <>
          <label className="switch-row">
            <input
              type="checkbox"
              checked={showCover}
              onChange={(e) => setShowCover(e.target.checked)}
            />
            <span>展示封面图片</span>
            <span className="switch-hint">
              {showCover ? "书架显示封面图" : "书架显示纯色封面"}
            </span>
          </label>

          <div className="pages-section">
            <div className="pages-section-head">
              <span className="settings-hint">
                图片（共 {pages.length} 页）· 点击设为封面，拖动调整顺序
              </span>
              <div className="pages-section-actions">
                {pendingDelete.size > 0 && (
                  <>
                    <button
                      type="button"
                      className="btn btn-quiet"
                      onClick={() => setPendingDelete(new Set())}
                    >
                      取消选择
                    </button>
                    <button
                      type="button"
                      className="btn btn-danger"
                      disabled={working}
                      onClick={() => void removePending()}
                    >
                      删除所选 {pendingDelete.size} 页
                    </button>
                  </>
                )}
                <button
                  type="button"
                  className="btn"
                  disabled={working}
                  onClick={() => void addImages()}
                >
                  ＋ 添加图片
                </button>
                <button
                  type="button"
                  className="btn"
                  disabled={working || pages.length === 0}
                  title="把这部漫画按当前顺序打包成 zip，保存到系统的下载目录"
                  onClick={() => void exportZip()}
                >
                  导出
                </button>
              </div>
            </div>

            {exporting && (
              <div className="settings-progress">
                <div className="busy-progress">
                  <div
                    className={`busy-progress-bar${
                      exporting.ratio === null ? " busy-progress-bar--indeterminate" : ""
                    }`}
                    style={
                      exporting.ratio === null
                        ? undefined
                        : { width: `${Math.round(exporting.ratio * 100)}%` }
                    }
                  />
                </div>
                <div className="busy-progress-text">{exporting.label}</div>
              </div>
            )}

            {pages.length > 0 ? (
              <div
                className="page-picker"
                ref={pickerRef}
                {...sorting.containerHandlers}
              >
                {pages.map((p, i) => {
                  const selected = cover === p;
                  const pending = pendingDelete.has(p);
                  const cls = [
                    "page-thumb",
                    selected && !pending ? "page-thumb-selected" : "",
                    pending ? "page-thumb-pending" : "",
                    sorting.dragPage === p ? "page-thumb-dragging" : "",
                    sorting.dragPage !== p && sorting.overIndex === i
                      ? "page-thumb-drop-before"
                      : "",
                    sorting.dragPage !== p &&
                    sorting.overIndex === pages.length &&
                    i === pages.length - 1
                      ? "page-thumb-drop-after"
                      : "",
                  ]
                    .filter(Boolean)
                    .join(" ");
                  return (
                    <div className="page-thumb-wrap" key={p}>
                      <button
                        type="button"
                        data-page={p}
                        className={cls}
                        onClick={() => {
                          if (!pending) setCover(p);
                        }}
                        title={`第 ${i + 1} 页${pending ? "（待删除）" : ""}`}
                        {...sorting.thumbHandlers}
                      >
                        <img
                          src={pageSrc(p)}
                          alt={`第 ${i + 1} 页`}
                          loading="lazy"
                          draggable={false}
                        />
                        <span className="page-thumb-index">{i + 1}</span>
                        {pending ? (
                          <span className="page-thumb-badge page-thumb-badge-danger">
                            待删除
                          </span>
                        ) : (
                          selected && (
                            <span className="page-thumb-badge">封面</span>
                          )
                        )}
                      </button>
                      <span
                        className="page-thumb-delete"
                        data-no-drag
                        title={pending ? "取消删除这一页" : "删除这一页"}
                        onClick={() => togglePending(p)}
                      >
                        {pending ? "↺" : "✕"}
                      </span>
                    </div>
                  );
                })}
              </div>
            ) : (
              <p className="settings-hint">
                这部漫画还没有图片，点「添加图片」加入。
              </p>
            )}
          </div>

          {dangerZone}
        </>
      ) : (
        <>
          <p className="settings-hint">
            文件夹在书架上以固定图标展示，暂无封面设置。
          </p>
          {dangerZone}
        </>
      )}
    </Modal>
  );
}
