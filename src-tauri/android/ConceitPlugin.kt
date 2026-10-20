package com.conceit.mangareader

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import androidx.activity.result.ActivityResult
import androidx.core.view.WindowInsetsCompat
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
 * Conceit 的 Android 原生桥。
 *
 * 为什么需要它：tauri-plugin-dialog 在 Android 上有两个硬伤——
 * 1. 没有目录选择器——JS 传 `directory: true` 时，插件 Rust 端直接返回
 *    FolderPickerNotImplemented（表现为「打开选择对话框失败」）；
 * 2. 文件选择器返回的不是路径——它返回的是 content:// URI 字符串，
 *    Rust 的 std::fs 无法读取（表现为「路径不存在」）。
 *
 * 这里用系统文档选择器（SAF）完成「选择 → 复制到应用缓存目录 → 返回真实路径」，
 * Rust 侧之后按普通文件夹 / zip 处理（见 src-tauri/src/mobile_import.rs）。
 * 复制时保留原始名称，导入后的漫画标题仍然是有意义的文件夹名 / 压缩包名。
 *
 * 除导入选择外，桌面端没有对应物的原生能力（退出应用、系统栏高度…）
 * 也统一放在这个插件里，由 desktop 存根给出同签名错误返回。
 *
 * 注意：ActivityResult 回调运行在主线程，复制整棵目录树可能很慢，
 * 因此复制放在后台线程做，完成后回到主线程 resolve，避免 ANR。
 */
@TauriPlugin
class ConceitPlugin(private val activity: Activity) : Plugin(activity) {

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
      } catch (e: Throwable) {
        rejectOnUi(invoke, e.message ?: "读取所选文件失败")
      }
    }.start()
  }

  /** 选择图片文件（可多选），复制进缓存目录，返回真实路径列表。
   *  给设置页「增加图片」用：选完后由 Rust 侧复制进书库漫画目录。 */
  @Command
  fun pickImages(invoke: Invoke) {
    val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
      addCategory(Intent.CATEGORY_OPENABLE)
      type = "image/*"
      putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true)
    }
    startActivityForResult(invoke, intent, "imagesPicked")
  }

  @ActivityCallback
  fun imagesPicked(invoke: Invoke, result: ActivityResult) {
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
    // 与 zip 选择相同：复制放后台线程，完成后回主线程 resolve
    Thread {
      try {
        val dir = prepareDir("picked-images", wipe = true)
        val paths = uris.mapNotNull { uri ->
          copyUriToFile(uri, dir, ensureZipExtension = false)
        }
        resolveOnUi(invoke) { pathsResponse(paths) }
      } catch (e: Throwable) {
        rejectOnUi(invoke, e.message ?: "读取所选图片失败")
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
        // 安卓导入是「SAF 拷进缓存 + Rust 再拷进书库」两份占用，拷到一半
        // 空间不足只会得到一条错误、白等几分钟。先按目录总大小预估，不够
        // 就提前拒绝；提供者不给文件大小时放弃预估、照常导入。
        val total = treeTotalSize(treeUri)
        if (total != null) {
          val usable = activity.cacheDir.usableSpace
          val need = total * 2 + 64L * 1024 * 1024
          if (usable < need) {
            rejectOnUi(
              invoke,
              "存储空间不足：导入约需 ${need / 1024 / 1024}MB，当前可用 ${usable / 1024 / 1024}MB"
            )
            return@Thread
          }
        }
        copyTree(treeUri, dest)
        resolveOnUi(invoke) { JSObject().put("path", dest.absolutePath) }
      } catch (e: Throwable) {
        // Throwable 而非 Exception：OOM、StackOverflowError 也要能转成一条
        // 错误提示，而不是未捕获异常直接杀掉进程（用户看到的就是闪退）
        rejectOnUi(invoke, e.message ?: "读取所选目录失败")
      }
    }.start()
  }

  /** 清理导入用的缓存目录（前端导入 / 增加图片完成后调用） */
  @Command
  fun clearImportCache(invoke: Invoke) {
    Thread {
      try {
        File(activity.cacheDir, "picked-import").deleteRecursively()
        File(activity.cacheDir, "tree-import").deleteRecursively()
        File(activity.cacheDir, "picked-images").deleteRecursively()
        resolveOnUi(invoke) { JSObject().put("cleared", true) }
      } catch (e: Throwable) {
        rejectOnUi(invoke, e.message ?: "清理导入缓存失败")
      }
    }.start()
  }

  /** 退出应用：安卓系统返回在书架根目录时由前端调用。
   *  finish() 与系统默认的「返回即退出」行为一致；书库状态均已实时落盘，
   *  无需额外清理。先 resolve 再 finish，让前端的 await 正常收尾。 */
  @Command
  fun exitApp(invoke: Invoke) {
    invoke.resolve()
    activity.finish()
  }

  /** 系统栏高度（px）：top = 状态栏，bottom = 导航栏。
   *  WebView 的 env(safe-area-inset-*) 在部分设备上取不到值（返回 0），
   *  顶栏 / 底栏 / 弹窗的避让改由前端查询这里的真实高度。 */
  @Command
  fun systemBarInsets(invoke: Invoke) {
    val rootInsets = activity.window?.decorView?.rootWindowInsets
    val insets = if (rootInsets != null) {
      WindowInsetsCompat.toWindowInsetsCompat(rootInsets)
    } else {
      WindowInsetsCompat.CONSUMED
    }
    invoke.resolve(
      JSObject()
        .put("top", insets.getInsets(WindowInsetsCompat.Type.statusBars()).top)
        .put("bottom", insets.getInsets(WindowInsetsCompat.Type.navigationBars()).bottom)
    )
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

  /** 深度优先遍历 SAF 目录树。
   *
   * 用显式栈而非递归：深层目录树会把线程栈爆掉，而 StackOverflowError 不是
   * Exception、catch 不住，未捕获的 Error 会直接让进程闪退。
   * visitDir 决定子目录对应的本地目录（大小预估时可原样透传、不建目录），
   * onFile 对每个文件回调（size = null 表示提供者不给出文件大小）。 */
  private inline fun walkTree(
    treeUri: Uri,
    dest: File,
    visitDir: (File, String) -> File = { dir, name -> File(dir, name).also { it.mkdirs() } },
    onFile: (Uri, File, Long?) -> Unit,
  ) {
    val stack = ArrayDeque<Pair<Uri, File>>()
    stack.addLast(
      DocumentsContract.buildChildDocumentsUriUsingTree(
        treeUri,
        DocumentsContract.getTreeDocumentId(treeUri)
      ) to dest
    )
    while (stack.isNotEmpty()) {
      val (childrenUri, dir) = stack.removeLast()
      activity.contentResolver.query(
        childrenUri,
        arrayOf(
          DocumentsContract.Document.COLUMN_DOCUMENT_ID,
          DocumentsContract.Document.COLUMN_MIME_TYPE,
          DocumentsContract.Document.COLUMN_DISPLAY_NAME,
          DocumentsContract.Document.COLUMN_SIZE
        ),
        null, null, null
      )?.use { cursor ->
        while (cursor.moveToNext()) {
          val docId = cursor.getString(0) ?: continue
          val mime = cursor.getString(1)
          val name = sanitizeName(cursor.getString(2) ?: docId)
          if (mime == DocumentsContract.Document.MIME_TYPE_DIR) {
            stack.addLast(
              DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, docId) to visitDir(dir, name)
            )
          } else {
            val size = if (cursor.isNull(3)) null else cursor.getLong(3)
            onFile(DocumentsContract.buildDocumentUriUsingTree(treeUri, docId), dir, size)
          }
        }
      }
    }
  }

  private fun copyTree(treeUri: Uri, dest: File) {
    walkTree(treeUri, dest) { docUri, dir, _ ->
      copyUriToFile(docUri, dir, ensureZipExtension = false)
    }
  }

  /** 统计目录树全部文件大小；任一文件大小未知时返回 null（放弃预估、直接放行） */
  private fun treeTotalSize(treeUri: Uri): Long? {
    var total = 0L
    walkTree(treeUri, File("/"), visitDir = { dir, _ -> dir }) { _, _, size ->
      size ?: return null
      total += size
    }
    return total
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
