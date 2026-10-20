import { useState } from "react";
import Modal from "./Modal";

interface Props {
  title: string;
  message: string;
  confirmText?: string;
  /** 危险操作用红色主按钮 */
  danger?: boolean;
  onConfirm: () => void | Promise<void>;
  onClose: () => void;
}

/** 二次确认弹窗（替代 window.confirm）。
 *
 *  为什么不用原生 confirm：安卓 WebView 没有实现 onJsConfirm，
 *  window.confirm 会直接返回 false——删除类操作会被静默取消。
 *  用应用内模态框保证桌面 / 安卓行为一致，样式也与其它弹窗统一。
 *  确认期间禁用按钮，防止重复提交。 */
export default function ConfirmDialog({
  title,
  message,
  confirmText = "确定",
  danger = false,
  onConfirm,
  onClose,
}: Props) {
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    setBusy(true);
    try {
      await onConfirm();
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
            className={`btn${danger ? " btn-danger" : " btn-primary"}`}
            disabled={busy}
            onClick={() => void submit()}
          >
            {busy ? "处理中…" : confirmText}
          </button>
        </>
      }
    >
      <p className="confirm-message">{message}</p>
    </Modal>
  );
}
