package com.conceit.mangareader

import android.app.Activity
import app.tauri.Logger
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject

/**
 * Tauri 插件命令的执行辅助。
 *
 * 三条跨命令的约定在这里只写一次：
 * 1. **resolve / reject 回主线程**——@Command 方法与 ActivityResult 回调都运行在
 *    主线程，但复制、查询、落盘等重活必须放后台；跨线程 resolve 会崩，统一包一层。
 * 2. **后台执行统一兜底**——整棵目录树的复制可能很慢（见 ConceitPlugin 的各命令），
 *    放后台线程做；catch Throwable 而非 Exception，OOM、StackOverflowError 也要
 *    能转成一条错误提示，而不是未捕获异常直接杀掉进程（用户看到的就是闪退）。
 * 3. **错误文案由调用方给**——每条命令知道自己该提示什么（「读取所选目录失败」
 *    还是「写入下载目录失败」），兜底只负责送达。
 */

/** 在主线程 resolve：payload 在 build 里现拼，避免在命令线程构造 JSObject */
internal fun Activity.resolveOnUi(invoke: Invoke, build: () -> JSObject) {
  runOnUiThread { invoke.resolve(build()) }
}

/** 在主线程 reject */
internal fun Activity.rejectOnUi(invoke: Invoke, message: String) {
  runOnUiThread { invoke.reject(message) }
}

/**
 * 后台执行一个命令任务。
 *
 * 正常结束由 work 自行 resolve；异常统一 reject 成 message（Throwable 的
 * message 为空时——部分框架异常如此——退回调用方给出的通用文案）。
 * onError 留给需要额外诊断日志的调用方（如 SAF 复制失败要记堆栈）。
 */
internal fun Activity.runBackground(
  invoke: Invoke,
  failMessage: String,
  onError: (Throwable) -> Unit = {},
  work: () -> Unit,
) {
  Thread {
    try {
      work()
    } catch (e: Throwable) {
      onError(e)
      rejectOnUi(invoke, e.message ?: failMessage)
    }
  }.start()
}

/**
 * 从 invoke 参数里取进度 Channel（前端不传时返回 null，进度条走不确定态）。
 *
 * 解析失败只记日志、不 reject：进度是可选增强，为它让整条命令失败不值得
 * ——与 setStatusBarStyle 解析参数失败即 reject 的策略差异是有意的：
 * 那边缺了参数命令本身没有意义，这边缺了通道命令照常完成。
 */
internal fun parseProgressChannel(invoke: Invoke): Channel? = try {
  invoke.parseArgs(ProgressArgs::class.java).channel
} catch (e: Throwable) {
  Logger.error("解析进度通道失败: ${e.message}")
  null
}
