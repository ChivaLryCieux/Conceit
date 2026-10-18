import { useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import Modal from "./ui/Modal";
import { updateItemSettings } from "../api";
import type { Item } from "../types";

interface Props {
  item: Item;
  /** 书库根目录（绝对路径） */
  root: string;
  onClose: () => void;
  onSaved: () => void | Promise<void>;
  onToast: (text: string, tone?: "info" | "error" | "success") => void;
}

/** 条目设置：名称、是否展示封面、选择漫画某一页作为封面 */
export default function ItemSettings({
  item,
  root,
  onClose,
  onSaved,
  onToast,
}: Props) {
  const [title, setTitle] = useState(item.title);
  const [showCover, setShowCover] = useState(item.show_cover);
  const [cover, setCover] = useState<string | undefined>(item.manga?.cover);
  const [busy, setBusy] = useState(false);

  const isManga = item.kind === "manga";
  const pages = item.manga?.pages ?? [];
  const mangaDir = item.manga?.dir ?? "";

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

  return (
    <Modal
      title={`设置 · ${item.title}`}
      onClose={onClose}
      wide={isManga && pages.length > 0}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            取消
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={busy || !title.trim()}
            onClick={() => void save()}
          >
            {busy ? "保存中…" : "保存"}
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

          {pages.length > 0 && (
            <>
              <p className="settings-hint">
                点击任意一页将其设为封面（共 {pages.length} 页）：
              </p>
              <div className="page-picker">
                {pages.map((p, i) => {
                  const selected = cover === p;
                  return (
                    <button
                      key={p}
                      type="button"
                      className={`page-thumb${
                        selected ? " page-thumb-selected" : ""
                      }`}
                      onClick={() => setCover(p)}
                      title={`第 ${i + 1} 页`}
                    >
                      <img
                        src={convertFileSrc(`${root}/${mangaDir}/${p}`)}
                        alt={`第 ${i + 1} 页`}
                        loading="lazy"
                        draggable={false}
                      />
                      <span className="page-thumb-index">{i + 1}</span>
                      {selected && <span className="page-thumb-badge">封面</span>}
                    </button>
                  );
                })}
              </div>
            </>
          )}
        </>
      ) : (
        <p className="settings-hint">
          文件夹在书架上以固定图标展示，暂无封面设置。
        </p>
      )}
    </Modal>
  );
}
