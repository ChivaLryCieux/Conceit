import { useEffect, useState } from "react";
import Modal from "./Modal";

interface Props {
  title?: string;
  label?: string;
  initial?: string;
  placeholder?: string;
  confirmText?: string;
  onConfirm: (value: string) => void | Promise<void>;
  onClose: () => void;
}

/** 单行输入弹窗（替代浏览器 prompt），用于新建文件夹等场景 */
export default function InputDialog({
  title = "新建文件夹",
  label = "名称",
  initial = "",
  placeholder = "请输入名称",
  confirmText = "创建",
  onConfirm,
  onClose,
}: Props) {
  const [value, setValue] = useState(initial);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      const el = document.getElementById("input-dialog-field") as HTMLInputElement | null;
      el?.focus();
      el?.select();
    }, 30);
    return () => window.clearTimeout(timer);
  }, []);

  const submit = async () => {
    const trimmed = value.trim();
    if (!trimmed) return;
    setBusy(true);
    try {
      await onConfirm(trimmed);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      title={title}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            取消
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={!value.trim() || busy}
            onClick={() => void submit()}
          >
            {busy ? "处理中…" : confirmText}
          </button>
        </>
      }
    >
      <label className="field">
        <span className="field-label">{label}</span>
        <input
          id="input-dialog-field"
          className="input"
          value={value}
          placeholder={placeholder}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void submit();
          }}
        />
      </label>
    </Modal>
  );
}
