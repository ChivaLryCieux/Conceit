import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { listen } from "@tauri-apps/api/event";
import Shelf from "./components/shelf/Shelf";
import Reader from "./components/reader/Reader";
import { fetchSnapshot, importSources } from "./api";
import type { Item, LibrarySnapshot, Toast } from "./types";

export default function App() {
  const [snapshot, setSnapshot] = useState<LibrarySnapshot | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  /** 文件夹栈：最后一项为当前所在文件夹 id；空 = 根目录 */
  const [folderStack, setFolderStack] = useState<string[]>([]);
  const [reading, setReading] = useState<Item | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);

  const reload = useCallback(async () => {
    try {
      const snap = await fetchSnapshot();
      setSnapshot(snap);
      setLoadError(null);
    } catch (e) {
      setLoadError(String(e));
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  // 启动时的后台缩略图补齐完成后，Rust 会发事件通知；刷新一次书架即可用上
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      try {
        const stop = await listen<number>("library:thumbnails-ready", () => {
          if (!cancelled) void reload();
        });
        if (cancelled) stop();
        else unlisten = stop;
      } catch {
        // 事件监听失败不影响主流程
      }
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [reload]);

  const pushToast = useCallback((text: string, tone: Toast["tone"] = "info") => {
    const id = Date.now() + Math.random();
    setToasts((list) => [...list, { id, text, tone }]);
    window.setTimeout(
      () => setToasts((list) => list.filter((t) => t.id !== id)),
      3600
    );
  }, []);

  const currentFolder = folderStack.length
    ? folderStack[folderStack.length - 1]
    : null;

  /** 拖拽文件/文件夹到窗口 = 直接导入 */
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      try {
        const stop = await getCurrentWebview().onDragDropEvent(async (event) => {
          if (cancelled) return;
          const payload = event.payload as { type: string; paths?: string[] };
          if (payload.type === "drop" && payload.paths?.length) {
            try {
              const items = await invoke<Item[]>("import_sources", {
                paths: payload.paths,
                targetFolder: currentFolder,
              });
              if (items.length > 0) {
                await reload();
                pushToast(`已导入 ${items.length} 部漫画`, "success");
              }
            } catch (e) {
              pushToast(`导入失败: ${e}`, "error");
            }
          }
        });
        if (cancelled) stop();
        else unlisten = stop;
      } catch {
        // 拖拽导入不可用时静默降级
      }
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [currentFolder, reload, pushToast]);

  /** 面包屑名称 */
  const breadcrumbs = useMemo(() => {
    const list: { id: string | null; title: string }[] = [
      { id: null, title: "全部书架" },
    ];
    for (const fid of folderStack) {
      const item = snapshot?.index.items.find((i) => i.id === fid);
      list.push({ id: fid, title: item?.title ?? "文件夹" });
    }
    return list;
  }, [folderStack, snapshot]);

  if (!snapshot) {
    return (
      <div className="app-shell">
        <div className="app-boot">
          {loadError ? (
            <>
              <p className="app-boot-error">书库加载失败</p>
              <pre>{loadError}</pre>
              <button className="btn" onClick={() => void reload()}>
                重试
              </button>
            </>
          ) : (
            <p>正在打开书架……</p>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="app-shell">
      <Shelf
        snapshot={snapshot}
        currentFolder={currentFolder}
        breadcrumbs={breadcrumbs}
        onEnterFolder={(id) => setFolderStack((s) => [...s, id])}
        onNavigateDepth={(depth) => setFolderStack((s) => s.slice(0, depth))}
        onOpenManga={setReading}
        onReload={reload}
        onToast={pushToast}
        onImport={async (paths) => {
          const items = await importSources(paths, currentFolder);
          await reload();
          return items;
        }}
      />
      {reading && (
        <Reader
          item={reading}
          onExit={() => setReading(null)}
          onToast={pushToast}
        />
      )}
      <div className="toast-stack">
        {toasts.map((t) => (
          <div key={t.id} className={`toast toast-${t.tone}`}>
            {t.text}
          </div>
        ))}
      </div>
    </div>
  );
}
