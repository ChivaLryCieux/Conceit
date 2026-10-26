package com.conceit.mangareader

import android.app.Activity
import android.content.ContentResolver
import android.content.ContentValues
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.DocumentsContract
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import android.util.Log
import androidx.activity.result.ActivityResult
import androidx.annotation.RequiresApi
import androidx.core.view.WindowInsetsCompat
import app.tauri.Logger
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File
import java.io.FileOutputStream
import java.io.OutputStream

/** 选择器命令的可选参数：前端传入的进度通道 */
@InvokeArg
internal class PickerArgs {
  var channel: Channel? = null
}

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
 * 导出方向同样绕不开这道墙：书库在应用私有目录里，Rust 把漫画打包成 zip
 * 后，由这里的 saveToDownloads 经 MediaStore（Android 10+）或 SAF
 * 「创建文档」（更低版本）写进系统的公共下载目录。
 *
 * 注意：ActivityResult 回调运行在主线程，复制整棵目录树可能很慢，
 * 因此复制放在后台线程做，完成后回到主线程 resolve，避免 ANR。
 */
@TauriPlugin
class ConceitPlugin(private val activity: Activity) : Plugin(activity) {

  /** 复制进度节流状态（reportCopy 用；同一时刻只有一个导入任务，简单记录即可） */
  private val reportLock = Any()
  private var reportLast = 0L

  /** 落盘进度节流状态（reportSave 用）：与 reportCopy 分开一把锁，
   *  导出的「落盘」段与导入的复制互不干扰 */
  private val saveReportLock = Any()
  private var saveReportLast = 0L

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
    val channel = parseChannel(invoke)
    // 复制可能很慢：放后台线程，完成后回主线程 resolve
    Thread {
      try {
        val dir = prepareDir("picked-import", wipe = true)
        val paths = ArrayList<String>()
        uris.forEachIndexed { i, uri ->
          val size = querySize(uri)
          val name = queryDisplayName(uri) ?: uri.lastPathSegment ?: "file-$i"
          var done = 0L
          val path = copyUriToFile(uri, dir, ensureZipExtension = true) { n ->
            done += n
            reportCopy(channel, i + 1, uris.size, name, done, size ?: 0L)
          }
          if (path != null) paths.add(path)
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
        val cacheRoot = File(activity.cacheDir, "tree-import")
        val cacheExisted = cacheRoot.exists()
        val dest = File(prepareDir("tree-import", wipe = true), treeDisplayName(treeUri))
        dest.mkdirs()
        Log.i(
          "ConceitImport",
          "SAF 目录选择返回 $treeUri; 缓存目录${if (cacheExisted) "有残留(已清空重建)" else "为空"} → ${dest.absolutePath}"
        )
        // 安卓导入是「SAF 拷进缓存 + Rust 再拷进书库」两份占用，拷到一半
        // 空间不足只会得到一条错误、白等几分钟。先按目录总大小预估，不够
        // 就提前拒绝；提供者不给文件大小时放弃预估、照常导入。
        val total = treeTotalSize(treeUri)
        if (total != null) {
          val usable = activity.cacheDir.usableSpace
          val need = total * 2 + 64L * 1024 * 1024
          if (usable < need) {
            Log.e("ConceitImport", "存储空间不足: 需 ${need / 1024 / 1024}MB, 可用 ${usable / 1024 / 1024}MB")
            rejectOnUi(
              invoke,
              "存储空间不足：导入约需 ${need / 1024 / 1024}MB，当前可用 ${usable / 1024 / 1024}MB"
            )
            return@Thread
          }
        }
        val channel = parseChannel(invoke)
        val title = dest.name
        var done = 0L
        var count = 0
        copyTree(treeUri, dest) { n ->
          done += n
          count += 1
          reportCopy(channel, 1, 1, title, done, total ?: 0L)
        }
        // 磁盘上的文件数多于本次复制数，说明缓存里有上一次导入的残留——
        // 这正是「一部漫画出现两份」的典型成因
        val onDisk = dest.walkTopDown().count { it.isFile }
        Log.i("ConceitImport", "SAF 复制完成: 本次 $count 个 / 磁盘共 $onDisk 个 / $done 字节 → ${dest.absolutePath}")
        resolveOnUi(invoke) { JSObject().put("path", dest.absolutePath) }
      } catch (e: Throwable) {
        // Throwable 而非 Exception：OOM、StackOverflowError 也要能转成一条
        // 错误提示，而不是未捕获异常直接杀掉进程（用户看到的就是闪退）
        Log.e("ConceitImport", "SAF 复制失败: ${e.javaClass.name}: ${e.message}", e)
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

  /** 导出命令的可选参数：待导出的 zip 路径、建议文件名、进度通道 */
  @InvokeArg
  internal class SaveFileArgs {
    var path: String? = null
    var fileName: String? = null
    var channel: Channel? = null
  }

  /** 把 Rust 侧打包好的 zip 写进系统「下载」目录，返回最终文件名。
   *
   * Android 10+ 走 MediaStore.Downloads：不需要任何权限，写完立刻出现在
   * 系统的下载应用里，是真正的「直接导出到下载目录」；更低版本没有这个
   * 集合，退回 SAF 的「创建文档」选择器（侧栏里就是「下载」），由用户
   * 点一下确认保存位置——SAF 也不需要存储权限。 */
  @Command
  fun saveToDownloads(invoke: Invoke) {
    val args = try {
      invoke.parseArgs(SaveFileArgs::class.java)
    } catch (e: Throwable) {
      invoke.reject("导出参数解析失败")
      return
    }
    val src = File(args.path ?: "")
    val fileName = args.fileName ?: "漫画.zip"
    if (!src.isFile) {
      invoke.reject("找不到待导出的文件")
      return
    }
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
      saveViaMediaStore(invoke, src, fileName, args.channel)
    } else {
      startCreateDocument(invoke, src, fileName)
    }
  }

  /** MediaStore 直写下载目录（API 29+）。IS_PENDING 先置 1，复制完成再归零，
   *  半截文件不会暴露给其他应用；中途失败把这条记录删掉，不留垃圾。 */
  @RequiresApi(Build.VERSION_CODES.Q)
  private fun saveViaMediaStore(invoke: Invoke, src: File, fileName: String, channel: Channel?) {
    Thread {
      try {
        val resolver = activity.contentResolver
        val name = uniqueDownloadsName(resolver, fileName)
        val values = ContentValues().apply {
          put(MediaStore.Downloads.DISPLAY_NAME, name)
          put(MediaStore.Downloads.MIME_TYPE, "application/zip")
          put(MediaStore.Downloads.IS_PENDING, 1)
        }
        val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
          ?: throw IllegalStateException("无法在下载目录创建文件")
        try {
          resolver.openOutputStream(uri)?.use { out ->
            copyFileWithProgress(src, out, channel)
          } ?: throw IllegalStateException("无法写入下载目录")
          val done = ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }
          resolver.update(uri, done, null, null)
          resolveOnUi(invoke) { JSObject().put("name", name) }
        } catch (e: Throwable) {
          runCatching { resolver.delete(uri, null, null) }
          throw e
        }
      } catch (e: Throwable) {
        rejectOnUi(invoke, e.message ?: "写入下载目录失败")
      }
    }.start()
  }

  /** 下载目录里已有同名文件时加 " (n)"，与桌面端 unique_dest_path 同一规则：
   *  导出是读者自己的资产，不覆盖上一次的结果。 */
  @RequiresApi(Build.VERSION_CODES.Q)
  private fun uniqueDownloadsName(resolver: ContentResolver, fileName: String): String {
    fun exists(name: String): Boolean {
      val cursor = resolver.query(
        MediaStore.Downloads.EXTERNAL_CONTENT_URI,
        arrayOf(MediaStore.Downloads._ID),
        "${MediaStore.Downloads.DISPLAY_NAME} = ?",
        arrayOf(name),
        null
      )
      cursor?.use { return it.moveToFirst() }
      return false
    }
    if (!exists(fileName)) return fileName
    val stem = fileName.substringBeforeLast('.', fileName)
    val ext = fileName.substringAfterLast('.', "")
    for (n in 1..999) {
      val candidate = if (ext.isEmpty()) "$stem ($n)" else "$stem ($n).$ext"
      if (!exists(candidate)) return candidate
    }
    val stamp = System.currentTimeMillis()
    return if (ext.isEmpty()) "$stem ($stamp)" else "$stem ($stamp).$ext"
  }

  /** Android 9 及以下的兜底：SAF 的「创建文档」选择器，默认建议文件名 */
  private fun startCreateDocument(invoke: Invoke, src: File, fileName: String) {
    val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
      addCategory(Intent.CATEGORY_OPENABLE)
      type = "application/zip"
      putExtra(Intent.EXTRA_TITLE, fileName)
    }
    startActivityForResult(invoke, intent, "createDocumentResult")
  }

  @ActivityCallback
  fun createDocumentResult(invoke: Invoke, result: ActivityResult) {
    if (result.resultCode != Activity.RESULT_OK) {
      invoke.reject("已取消导出")
      return
    }
    val uri = result.data?.data
    if (uri == null) {
      invoke.reject("没有选择保存位置")
      return
    }
    val args = try {
      invoke.parseArgs(SaveFileArgs::class.java)
    } catch (e: Throwable) {
      invoke.reject("导出参数解析失败")
      return
    }
    val src = File(args.path ?: "")
    val channel = args.channel
    Thread {
      try {
        if (!src.isFile) throw IllegalStateException("找不到待导出的文件")
        val name = sanitizeName(queryDisplayName(uri) ?: args.fileName ?: "漫画.zip")
        activity.contentResolver.openOutputStream(uri)?.use { out ->
          copyFileWithProgress(src, out, channel)
        } ?: throw IllegalStateException("无法写入所选位置")
        resolveOnUi(invoke) { JSObject().put("name", name) }
      } catch (e: Throwable) {
        rejectOnUi(invoke, e.message ?: "写入所选位置失败")
      }
    }.start()
  }

  /** 把文件复制进输出流，按 8KB 块回调进度（节流在 reportSave 里做）。
   *  与 copyUriToFile 同样不用 InputStream.copyTo：单个大 zip 会让进度条
   *  长时间停在 0 再一步跳到 100%。 */
  private fun copyFileWithProgress(src: File, out: OutputStream, channel: Channel?) {
    val buffer = ByteArray(64 * 1024)
    var done = 0L
    val total = src.length()
    src.inputStream().use { input ->
      out.use { output ->
        while (true) {
          val read = input.read(buffer)
          if (read < 0) break
          output.write(buffer, 0, read)
          done += read
          reportSave(channel, done, total)
        }
      }
    }
  }

  /** 落盘进度上报：约 10 次/秒，字段与 Rust 侧 ExportProgress 一致。
   *  与 reportCopy 分开一把锁：导出的「落盘」段与导入的复制互不干扰。 */
  private fun reportSave(channel: Channel?, doneBytes: Long, totalBytes: Long) {
    if (channel == null) return
    val now = System.currentTimeMillis()
    synchronized(saveReportLock) {
      if (now - saveReportLast < 100) return
      saveReportLast = now
    }
    channel.send(
      JSObject()
        .put("phase", "save")
        .put("doneBytes", doneBytes)
        .put("totalBytes", totalBytes)
    )
  }

  // ---------- 内部实现 ----------

  private fun pathsResponse(paths: List<String>): JSObject {
    val items: MutableList<String?> = ArrayList(paths)
    return JSObject().put("paths", JSArray.from(items.toTypedArray()))
  }

  /** 从 invoke 参数里取进度 Channel（前端不传时返回 null，进度条走不确定态） */
  private fun parseChannel(invoke: Invoke): Channel? = try {
    invoke.parseArgs(PickerArgs::class.java).channel
  } catch (e: Throwable) {
    Logger.error("解析进度通道失败: ${e.message}")
    null
  }

  /** 复制进度节流上报：约 10 次/秒，字段与 Rust 侧 ImportProgress 一致。
   *  doneBytes 由调用方按「当前这一部」累计（一部目录或一个 zip 各自从 0 起算）。 */
  private fun reportCopy(
    channel: Channel?,
    index: Int,
    total: Int,
    title: String,
    doneBytes: Long,
    totalBytes: Long
  ) {
    if (channel == null) return
    val now = System.currentTimeMillis()
    synchronized(reportLock) {
      if (now - reportLast < 100) return
      reportLast = now
    }
    // send 放在锁外：它会走 JNI 到 Rust，没必要让整个复制过程串行等待
    channel.send(
      JSObject()
        .put("phase", "copy")
        .put("index", index)
        .put("total", total)
        .put("title", title)
        .put("doneBytes", doneBytes)
        .put("totalBytes", totalBytes)
    )
  }

  /** 查询 content:// URI 的文件大小；提供者不给时返回 null */
  private fun querySize(uri: Uri): Long? {
    val projection = arrayOf(OpenableColumns.SIZE)
    activity.contentResolver.query(uri, projection, null, null, null)?.use { cursor ->
      if (cursor.moveToFirst()) {
        val idx = cursor.getColumnIndex(OpenableColumns.SIZE)
        if (idx >= 0 && !cursor.isNull(idx)) return cursor.getLong(idx)
      }
    }
    return null
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

  /** 把 content:// URI 的内容写成一个真实文件，返回其绝对路径（失败返回 null）。
   *  onBytes 按 8KB 块回调本次搬运的字节数（进度条用，节流在 reportCopy 里做）。
   *  不用 InputStream.copyTo：那个重载不给中间进度，单个大文件会让进度条
   *  长时间停在 0 再一步跳到 100%。 */
  private fun copyUriToFile(
    uri: Uri,
    dir: File,
    ensureZipExtension: Boolean,
    onBytes: (Long) -> Unit = {}
  ): String? {
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
    val buffer = ByteArray(64 * 1024)
    stream.use { input ->
      FileOutputStream(out).use { output ->
        while (true) {
          val read = input.read(buffer)
          if (read < 0) break
          output.write(buffer, 0, read)
          onBytes(read.toLong())
        }
      }
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

  private fun copyTree(treeUri: Uri, dest: File, onFile: (Long) -> Unit = {}) {
    walkTree(treeUri, dest) { docUri, dir, _ ->
      copyUriToFile(docUri, dir, ensureZipExtension = false) { n -> onFile(n) }
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
