import { useCallback, useEffect, useState } from "react";
import Modal from "./ui/Modal";
import ConfirmDialog from "./ui/ConfirmDialog";
import {
  libraryHealth,
  libraryHealthClean,
  rescanMangaPages,
  type HealthCleanupTarget,
} from "../api";
import type { DirStat, HealthReport } from "../types";
import type { Toast } from "../types";

interface Props {
  onClose: () => void;
  /** 清理 / 重新扫描后通知书架刷新 */
  onChanged: () => void | Promise<void>;
  onToast: (text: string, tone?: Toast["tone"]) => void;
}

/** 待确认的破坏性操作 */
interface PendingAction {
  /** 确认框里说明这次要删什么 */
  message: string;
  targets: HealthCleanupTarget[];
}

function formatBytes(bytes: number): string {
  if (bytes <= 0) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function formatTime(unixSeconds: number): string {
  if (!unixSeconds) return "时间未知";
  const d = new Date(unixSeconds * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(
    d.getHours()
  )}:${pad(d.getMinutes())}`;
}

function formatPercent(progress: number): string {
  return `${Math.round(progress * 100)}%`;
}

/** 一行磁盘实况：文件数 / 图片数 / 字节，附文件名样本 */
function DirStatLine({ stat }: { stat: DirStat }) {
  return (
    <>
      <span className="health-stat">
        {stat.files} 个文件 / {stat.images} 张图 / {formatBytes(stat.bytes)}
      </span>
      {stat.sample.length > 0 && (
        <span className="health-sample" title={stat.sample.join("、")}>
          {stat.sample.join("、")}
          {stat.files > stat.sample.length ? " …" : ""}
        </span>
      )}
    </>
  );
}

/** 书库体检：索引与磁盘实况对比。
 *
 *  导入是「先复制文件、后写索引」两步，中途被杀会留下对不上的状态
 *  （孤儿目录 / 索引条目没了目录 / 重复导入两部一样的漫画），而用户
 *  看不到应用内部目录，只能这样把真相翻到明面上并清理。 */
export default function LibraryHealth({
  onClose,
  onChanged,
  onToast,
}: Props) {
  const [report, setReport] = useState<HealthReport | null>(null);
  const [scanning, setScanning] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<PendingAction | null>(null);
  const [working, setWorking] = useState(false);
  const [showDiskStats, setShowDiskStats] = useState(false);

  const scan = useCallback(async () => {
    setScanning(true);
    setError(null);
    try {
      setReport(await libraryHealth());
    } catch (e) {
      setError(`体检失败：${e}`);
    } finally {
      setScanning(false);
    }
  }, []);

  useEffect(() => {
    void scan();
  }, [scan]);

  /** 执行清理（已由用户确认），完成后自动重新体检 */
  async function runCleanup(action: PendingAction) {
    setWorking(true);
    try {
      const n = await libraryHealthClean(action.targets);
      onToast(`已清理 ${n} 项`, "success");
      setPending(null);
      await onChanged();
      await scan();
    } catch (e) {
      onToast(`清理失败：${e}`, "error");
    } finally {
      setWorking(false);
    }
  }

  async function rescan(id: string, title: string) {
    setWorking(true);
    try {
      await rescanMangaPages(id);
      onToast(`已按磁盘重新扫描《${title}》`, "success");
      await onChanged();
      await scan();
    } catch (e) {
      onToast(`重新扫描失败：${e}`, "error");
    } finally {
      setWorking(false);
    }
  }

  const issueCount = report
    ? report.missing_dirs.length +
      report.orphan_dirs.length +
      report.duplicates.length +
      report.page_drift.length
    : 0;

  return (
    <Modal
      title="书库体检"
      onClose={onClose}
      wide
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            关闭
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={scanning || working}
            onClick={() => void scan()}
          >
            {scanning ? "扫描中…" : "重新体检"}
          </button>
        </>
      }
    >
      {scanning && !report && <p className="settings-hint">正在扫描书库…</p>}
      {error && <p className="settings-hint">{error}</p>}

      {report && (
        <>
          <p className="health-summary">
            索引 {report.indexed} 部 · 磁盘 {report.on_disk} 个目录 ·{" "}
            {issueCount === 0 ? "未发现问题" : `发现 ${issueCount} 处需要关注`}
          </p>

          {issueCount === 0 && (
            <p className="health-ok">
              索引里的每部漫画都能在磁盘上找到，也没有多余目录或重复导入。
            </p>
          )}

          {/* 索引有条目、目录丢了 */}
          {report.missing_dirs.length > 0 && (
            <section className="health-section">
              <h3 className="health-title">
                索引有条目、目录丢失（{report.missing_dirs.length}）
              </h3>
              <p className="settings-hint">
                这些漫画的图片目录已经不在磁盘上（导入没写完就中断，或文件被外部清掉），
                在书架上点进去也是空的。移除条目只影响书架，不动其它漫画。
              </p>
              {report.missing_dirs.map((m) => (
                <div className="health-row" key={m.id}>
                  <div className="health-row-main">
                    <span className="health-id">{m.id}</span>
                    <span className="health-name">《{m.title}》</span>
                    <span className="health-stat">索引记录 {m.pages} 页</span>
                  </div>
                  <button
                    type="button"
                    className="btn btn-danger"
                    disabled={working}
                    onClick={() =>
                      setPending({
                        message: `把《${m.title}》从书架移除吗？它的图片目录已经不在磁盘上，此操作不可恢复。`,
                        targets: [{ id: m.id, deleteFiles: false }],
                      })
                    }
                  >
                    从书架移除
                  </button>
                </div>
              ))}
            </section>
          )}

          {/* 目录在、索引没有 */}
          {report.orphan_dirs.length > 0 && (
            <section className="health-section">
              <h3 className="health-title">
                磁盘上有目录、索引里没有（{report.orphan_dirs.length}）
              </h3>
              <p className="settings-hint">
                导入死在「复制文件」与「写索引」之间就会留下这种目录：书架上看不见，
                却一直占着空间。删除前可以先看下面的文件名确认内容。
              </p>
              {report.orphan_dirs.map((d) => (
                <div className="health-row" key={d.id}>
                  <div className="health-row-main">
                    <span className="health-id">{d.id}</span>
                    <span className="health-name">（索引中没有）</span>
                    <DirStatLine stat={d} />
                  </div>
                  <button
                    type="button"
                    className="btn btn-danger"
                    disabled={working}
                    onClick={() =>
                      setPending({
                        message: `删除目录 ${d.id}（${d.files} 个文件 / ${formatBytes(
                          d.bytes
                        )}）吗？它不在索引里，书架上看不到，删除后无法恢复。`,
                        targets: [{ id: d.id, deleteFiles: true }],
                      })
                    }
                  >
                    删除目录
                  </button>
                </div>
              ))}
            </section>
          )}

          {/* 疑似重复 */}
          {report.duplicates.length > 0 && (
            <section className="health-section">
              <h3 className="health-title">
                疑似重复（{report.duplicates.length} 组）
              </h3>
              <p className="settings-hint">
                {report.duplicates[0].reason === "pages_identical"
                  ? "页面完全相同，基本可以确定是同一部漫画导了两次——常见于「导入已经落盘、但界面没刷新」时又导了一遍。"
                  : "同名且页数相同，但文件名不同（重复导入前后改过名）。"}
                想保留哪份就删掉另一份；有阅读进度的那份通常是你要的。
              </p>
              {report.duplicates.map((group) => (
                <div className="health-group" key={group.title}>
                  <div className="health-group-head">
                    《{group.title}》· {group.members.length} 份
                  </div>
                  {group.members.map((m) => (
                    <div className="health-row" key={m.id}>
                      <div className="health-row-main">
                        <span className="health-id">{m.id}</span>
                        <span className="health-stat">
                          索引 {m.pages} 页 / 磁盘 {m.images} 张图 /{" "}
                          {formatBytes(m.bytes)}
                        </span>
                        <span className="health-stat">
                          {formatTime(m.added_at)} · 进度{" "}
                          {formatPercent(m.progress)}
                        </span>
                        {m.dir_missing && (
                          <span className="health-stat health-stat-warn">
                            目录已丢失
                          </span>
                        )}
                      </div>
                      <button
                        type="button"
                        className="btn btn-danger"
                        disabled={working}
                        onClick={() =>
                          setPending({
                            message: `删除《${group.title}》的 ${m.id} 这份吗？连磁盘目录一起删${m.dir_missing ? "（目录已丢失，只会移除书架条目）" : ""
                              }，无法恢复。`,
                            targets: [{ id: m.id, deleteFiles: true }],
                          })
                        }
                      >
                        删除这一份
                      </button>
                    </div>
                  ))}
                </div>
              ))}
            </section>
          )}

          {/* 索引页数与磁盘不一致 */}
          {report.page_drift.length > 0 && (
            <section className="health-section">
              <h3 className="health-title">
                索引页数与磁盘不一致（{report.page_drift.length}）
              </h3>
              <p className="settings-hint">
                文件被外部增删、或导入中途死去又复用同一个 id
                把内容合并进旧目录，都会出现这种偏差。按磁盘重新扫描即可修正。
              </p>
              {report.page_drift.map((d) => (
                <div className="health-row" key={d.id}>
                  <div className="health-row-main">
                    <span className="health-id">{d.id}</span>
                    <span className="health-name">《{d.title}》</span>
                    <span className="health-stat">
                      索引 {d.indexed} 页 / 磁盘 {d.on_disk} 张图
                    </span>
                  </div>
                  <button
                    type="button"
                    className="btn"
                    disabled={working}
                    onClick={() => void rescan(d.id, d.title)}
                  >
                    按磁盘重新扫描
                  </button>
                </div>
              ))}
            </section>
          )}

          {/* 磁盘实况 */}
          <section className="health-section">
            <button
              type="button"
              className="health-disclosure"
              onClick={() => setShowDiskStats((v) => !v)}
            >
              {showDiskStats ? "▾" : "▸"} 磁盘实况（{report.disk_stats.length}{" "}
              个目录里实际有什么）
            </button>
            {showDiskStats && (
              <div className="health-disk-list">
                {report.disk_stats.map((d) => (
                  <div className="health-disk-row" key={d.id}>
                    <span className="health-id">{d.id}</span>
                    <span className="health-name">
                      {d.title ? `《${d.title}》` : "（索引中没有）"}
                    </span>
                    <DirStatLine stat={d} />
                  </div>
                ))}
              </div>
            )}
          </section>
        </>
      )}

      {pending && (
        <ConfirmDialog
          title="确认清理"
          message={pending.message}
          confirmText="删除"
          danger
          onConfirm={() => runCleanup(pending)}
          onClose={() => setPending(null)}
        />
      )}
    </Modal>
  );
}
