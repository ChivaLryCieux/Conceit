package com.conceit.mangareader

import app.tauri.plugin.Channel
import app.tauri.plugin.JSObject

/**
 * 带节流的进度上报（约 10 次/秒）。
 *
 * 导入的 SAF 复制与导出的落盘复制都按字节回调进度，字段分别与 Rust 侧
 * ImportProgress / ExportProgress 一致。两边的节流逻辑原先各写了一份
 * （reportCopy / reportSave，各配一把锁一个时间戳，存在插件实例上）——
 * 这里合并成一个类：每条业务流各持一个实例，同一时刻只有一个任务，
 * 实例级状态即可，无需共享，也就不需要那两把锁。
 *
 * channel 为 null（前端没传）时全部静默跳过：进度条走不确定态，功能不受影响。
 */
internal class ThrottledProgress(private val channel: Channel?) {

  private val lock = Any()
  private var last = 0L

  /**
   * 上报一帧进度；fields 在节流判定之后、发送之前拼装。
   * send 放在锁外：它会走 JNI 到 Rust，没必要让整个复制过程串行等待。
   */
  fun send(fields: JSObject.() -> Unit) {
    if (channel == null) return
    val now = System.currentTimeMillis()
    synchronized(lock) {
      if (now - last < 100) return
      last = now
    }
    channel.send(JSObject().apply(fields))
  }
}
