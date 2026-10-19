package com.conceit.mangareader

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import androidx.activity.result.ActivityResult
import app.tauri.Logger
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File
import java.io.FileOutputStream

/**
 * Conceit 的 Android 导入桥。
 *
 * 为什么需要它：
 * 1. tauri-plugin-dialog 在 Android 上没有目录选择器——JS 传 `directory: true` 时，
 *    插件 Rust 端直接返回 FolderPickerNotImplemented（表现为「打开选择对话框失败」）；
 * 2. 它的文件选择器返回的是 content:// URI 字符串，而不是文件系统路径，
 *    Rust 的 std::fs 无法读取（表现为「路径不存在」）。
 *
 * 这里用系统文档选择器（SAF）完成「选择 → 复制到应用缓存目录 → 返回真实路径」，
 * Rust 侧之后按普通文件夹 / zip 处理（见 src-tauri/src/mobile_import.rs）。
 * 复制时保留原始名称，导入后的漫画标题仍然是有意义的文件夹名 / 压缩包名。
 *
 * 注意：ActivityResult 回调运行在主线程，复制整棵目录树可能很慢，
 * 因此复制放在后台线程做，完成后回到主线程 resolve，避免 ANR。
 */
@TauriPlugin
class ImportPlugin(private val activity: Activity) : Plugin(activity) {

  /** 选择 zip 文件（可多选），复制进缓存目录，返回真实路径列表 */
  @Command
  fun pickZipFiles(invoke: Invoke) {
    val mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension("zip")
      ?: "application/zip"
    val intent = Intent(Intent.ACTION_GET_CONTENT).apply {
      addCategory(Intent.CATEGORY_OPENABLE)
      type = "*/*"
      putExtra(Intent.EXTRA_MIME_TYPES, arrayOf(mime))
      putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true)
    }
    startActivityForResult(invoke, intent, "zipPicked")
  }

  @ActivityCallback
  fun zipPicked(invoke: Invoke, result: ActivityResult) {
    if (result.resultCode != Activity.RESULT_OK) {
      invoke.resolve(pathsResponse(emptyList()))
      return
    }
    val data = result.data
    val uris: MutableList<Uri> = ArrayList()
    if (data?.clipData != null) {
      for (i in 0 until data.clipData!!.itemCount) {
        uris.add(data.clipData!!.getItemAt(i).uri)
      }
    } else {
      data?.data?.let { uris.add(it) }
    }
    // 复制可能很慢：放后台线程，完成后回主线程 resolve
    Thread {
      try {
        val dir = prepareDir("picked-import", wipe = true)
        val paths = uris.mapNotNull { uri ->
          copyUriToFile(uri, dir, ensureZipExtension = true)
        }
        resolveOnUi(invoke) { pathsResponse(paths) }
      } catch (e: Exception) {
        rejectOnUi(invoke, e.message ?: "读取所选文件失败")
      }
    }.start()
  }

  /** 选择目录（SAF 树选择器），整棵树复制进缓存目录，返回目录路径 */
  @Command
  fun pickDirectory(invoke: Invoke) {
    val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
      addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    }
    startActivityForResult(invoke, intent, "directoryPicked")
  }

  @ActivityCallback
  fun directoryPicked(invoke: Invoke, result: ActivityResult) {
    if (result.resultCode != Activity.RESULT_OK) {
      invoke.resolve(JSObject())
      return
    }
    val treeUri = result.data?.data
    if (treeUri == null) {
      invoke.resolve(JSObject())
      return
    }
    Thread {
      try {
        // 目录名取自所选文件夹的显示名，导入后的漫画标题才有意义
        val dest = File(prepareDir("tree-import", wipe = true), treeDisplayName(treeUri))
        dest.mkdirs()
        copyTree(treeUri, dest)
        resolveOnUi(invoke) { JSObject().put("path", dest.absolutePath) }
      } catch (e: Exception) {
        rejectOnUi(invoke, e.message ?: "读取所选目录失败")
      }
    }.start()
  }

  /** 清理导入用的缓存目录（前端导入完成后调用） */
  @Command
  fun clearImportCache(invoke: Invoke) {
    Thread {
      try {
        File(activity.cacheDir, "picked-import").deleteRecursively()
        File(activity.cacheDir, "tree-import").deleteRecursively()
        resolveOnUi(invoke) { JSObject().put("cleared", true) }
      } catch (e: Exception) {
        rejectOnUi(invoke, e.message ?: "清理导入缓存失败")
      }
    }.start()
  }

  // ---------- 内部实现 ----------

  private fun pathsResponse(paths: List<String>): JSObject {
    val items: MutableList<String?> = ArrayList(paths)
    return JSObject().put("paths", JSArray.from(items.toTypedArray()))
  }

  private fun resolveOnUi(invoke: Invoke, build: () -> JSObject) {
    activity.runOnUiThread { invoke.resolve(build()) }
  }

  private fun rejectOnUi(invoke: Invoke, message: String) {
    activity.runOnUiThread { invoke.reject(message) }
  }

  private fun prepareDir(name: String, wipe: Boolean = false): File {
    val dir = File(activity.cacheDir, name)
    if (wipe) dir.deleteRecursively()
    dir.mkdirs()
    return dir
  }

  /** 把 content:// URI 的内容写成一个真实文件，返回其绝对路径（失败返回 null） */
  private fun copyUriToFile(uri: Uri, dir: File, ensureZipExtension: Boolean): String? {
    var name = sanitizeName(
      queryDisplayName(uri) ?: uri.lastPathSegment ?: "file-${System.currentTimeMillis()}"
    )
    // 少数提供者查不到显示名，回退值可能形如 "msf:42"（含点但不是 .zip）；
    // 导入端按扩展名判断 zip，这里统一保证后缀正确
    if (ensureZipExtension && !name.endsWith(".zip", ignoreCase = true)) name = "$name.zip"
    val out = uniqueFile(dir, name)
    val stream = activity.contentResolver.openInputStream(uri)
    if (stream == null) {
      Logger.error("无法打开所选文件: $uri")
      return null
    }
    stream.use { input ->
      FileOutputStream(out).use { output -> input.copyTo(output) }
    }
    return out.absolutePath
  }

  /** 同名文件加序号，避免一次多选时互相覆盖 */
  private fun uniqueFile(dir: File, name: String): File {
    var candidate = File(dir, name)
    if (!candidate.exists()) return candidate
    val stem = name.substringBeforeLast('.')
    val ext = name.substringAfterLast('.', "")
    var n = 2
    while (candidate.exists()) {
      candidate = if (ext.isEmpty()) File(dir, "$stem-$n") else File(dir, "$stem-$n.$ext")
      n += 1
    }
    return candidate
  }

  private fun copyTree(treeUri: Uri, dest: File) {
    val rootId = DocumentsContract.getTreeDocumentId(treeUri)
    walkTree(treeUri, DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, rootId), dest)
  }

  private fun walkTree(treeUri: Uri, childrenUri: Uri, dest: File) {
    val projection = arrayOf(
      DocumentsContract.Document.COLUMN_DOCUMENT_ID,
      DocumentsContract.Document.COLUMN_MIME_TYPE,
      DocumentsContract.Document.COLUMN_DISPLAY_NAME
    )
    activity.contentResolver.query(childrenUri, projection, null, null, null)?.use { cursor ->
      while (cursor.moveToNext()) {
        val docId = cursor.getString(0) ?: continue
        val mime = cursor.getString(1)
        val name = sanitizeName(cursor.getString(2) ?: docId)
        if (mime == DocumentsContract.Document.MIME_TYPE_DIR) {
          val sub = File(dest, name)
          sub.mkdirs()
          walkTree(
            treeUri,
            DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, docId),
            sub
          )
        } else {
          val docUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, docId)
          copyUriToFile(docUri, dest, ensureZipExtension = false)
        }
      }
    }
  }

  private fun treeDisplayName(treeUri: Uri): String {
    val docId = DocumentsContract.getTreeDocumentId(treeUri)
    val docUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, docId)
    return sanitizeName(queryDisplayName(docUri) ?: docId.substringAfterLast(':'))
  }

  private fun queryDisplayName(uri: Uri): String? {
    val projection = arrayOf(OpenableColumns.DISPLAY_NAME)
    activity.contentResolver.query(uri, projection, null, null, null)?.use { cursor ->
      if (cursor.moveToFirst()) {
        val idx = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
        if (idx >= 0) return cursor.getString(idx)
      }
    }
    return null
  }

  /**
   * 清洗文档提供者返回的名称：路径分隔符、NUL 与前导点都可能让文件被写到
   * 缓存目录之外（如 "../../xxx"），统一替换掉。
   */
  private fun sanitizeName(raw: String): String {
    val cleaned = raw
      .replace('/', '_')
      .replace('\\', '_')
      .replace('\u0000', '_')
      .trim()
      .trimStart('.')
    return cleaned.ifEmpty { "unnamed" }
  }
}
