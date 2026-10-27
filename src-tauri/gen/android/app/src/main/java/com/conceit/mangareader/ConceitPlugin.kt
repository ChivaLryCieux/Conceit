package com.conceit.mangareader

import android.app.Activity
import android.content.ContentValues
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.MediaStore
import android.webkit.MimeTypeMap
import android.util.Log
import androidx.activity.result.ActivityResult
import androidx.annotation.RequiresApi
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import kotlin.math.roundToInt
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
import java.io.OutputStream

/** 选择器命令的可选参数：前端传入的进度通道 */
@InvokeArg
internal class PickerArgs {
  var channel: Channel? = null
}

/** setStatusBarStyle 的参数：状态栏图标是否用深色（浅色背景上） */
@InvokeArg
internal class StatusBarArgs {
  var light: Boolean = true
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
 * 因此复制放在后台线程做，完成后回到主线程 resolve，避免 ANR——
 * 这套「后台执行 + 统一兜错 + 回主线程」的骨架见 InvokeExt.kt。
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
    val progress = ThrottledProgress(parseChannel(invoke))
    // 复制可能很慢：放后台线程，完成后回主线程 resolve
    activity.runBackground(invoke, "读取所选文件失败") {
      val dir = ImportCache.PICKED_IMPORT.prepare(activity, wipe = true)
      val paths = ArrayList<String>()
      uris.forEachIndexed { i, uri ->
        val size = activity.contentResolver.querySize(uri)
        val name = activity.contentResolver.queryDisplayName(uri)
          ?: uri.lastPathSegment ?: "file-$i"
        var done = 0L
        val path = activity.contentResolver.copyUriToFile(uri, dir, ensureZipExtension = true) { n ->
          done += n
          progress.send {
            put("phase", "copy")
            put("index", i + 1)
            put("total", uris.size)
            put("title", name)
            put("doneBytes", done)
            put("totalBytes", size ?: 0L)
          }
        }
        if (path != null) paths.add(path)
      }
      activity.resolveOnUi(invoke) { pathsResponse(paths) }
    }
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
    activity.runBackground(invoke, "读取所选图片失败") {
      val dir = ImportCache.PICKED_IMAGES.prepare(activity, wipe = true)
      val paths = uris.mapNotNull { uri ->
        activity.contentResolver.copyUriToFile(uri, dir, ensureZipExtension = false)
      }
      activity.resolveOnUi(invoke) { pathsResponse(paths) }
    }
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
    activity.runBackground(
      invoke,
      "读取所选目录失败",
      onError = {
        Log.e("ConceitImport", "SAF 复制失败: ${it.javaClass.name}: ${it.message}", it)
      },
    ) {
      // 目录名取自所选文件夹的显示名，导入后的漫画标题才有意义
      val cacheRoot = ImportCache.TREE_IMPORT.dir(activity)
      val cacheExisted = cacheRoot.exists()
      val dest = File(
        ImportCache.TREE_IMPORT.prepare(activity, wipe = true),
        activity.contentResolver.treeDisplayName(treeUri)
      )
      dest.mkdirs()
      Log.i(
        "ConceitImport",
        "SAF 目录选择返回 $treeUri; 缓存目录${if (cacheExisted) "有残留(已清空重建)" else "为空"} → ${dest.absolutePath}"
      )
      // 安卓导入是「SAF 拷进缓存 + Rust 再拷进书库」两份占用，拷到一半
      // 空间不足只会得到一条错误、白等几分钟。先按目录总大小预估，不够
      // 就提前拒绝；提供者不给文件大小时放弃预估、照常导入。
      val total = activity.contentResolver.treeTotalSize(treeUri)
      if (total != null) {
        val usable = activity.cacheDir.usableSpace
        val need = total * 2 + 64L * 1024 * 1024
        if (usable < need) {
          Log.e("ConceitImport", "存储空间不足: 需 ${need / 1024 / 1024}MB, 可用 ${usable / 1024 / 1024}MB")
          activity.rejectOnUi(
            invoke,
            "存储空间不足：导入约需 ${need / 1024 / 1024}MB，当前可用 ${usable / 1024 / 1024}MB"
          )
          return@runBackground
        }
      }
      val progress = ThrottledProgress(parseChannel(invoke))
      val title = dest.name
      var done = 0L
      var count = 0
      activity.contentResolver.copyTree(treeUri, dest) { n ->
        done += n
        count += 1
        progress.send {
          put("phase", "copy")
          put("index", 1)
          put("total", 1)
          put("title", title)
          put("doneBytes", done)
          put("totalBytes", total ?: 0L)
        }
      }
      // 磁盘上的文件数多于本次复制数，说明缓存里有上一次导入的残留——
      // 这正是「一部漫画出现两份」的典型成因
      val onDisk = dest.walkTopDown().count { it.isFile }
      Log.i("ConceitImport", "SAF 复制完成: 本次 $count 个 / 磁盘共 $onDisk 个 / $done 字节 → ${dest.absolutePath}")
      activity.resolveOnUi(invoke) { JSObject().put("path", dest.absolutePath) }
    }
  }

  /** 清理导入用的缓存目录（前端导入 / 增加图片完成后调用） */
  @Command
  fun clearImportCache(invoke: Invoke) {
    activity.runBackground(invoke, "清理导入缓存失败") {
      ImportCache.entries.forEach { it.dir(activity).deleteRecursively() }
      activity.resolveOnUi(invoke) { JSObject().put("cleared", true) }
    }
  }

  /** 退出应用：安卓系统返回在书架根目录时由前端调用。
   *  finish() 与系统默认的「返回即退出」行为一致；书库状态均已实时落盘，
   *  无需额外清理。先 resolve 再 finish，让前端的 await 正常收尾。 */
  @Command
  fun exitApp(invoke: Invoke) {
    invoke.resolve()
    activity.finish()
  }

  /** 系统栏高度（CSS px，即 dp）：top = 状态栏，bottom = 导航栏。
   *
   *  WebView 的 env(safe-area-inset-*) 在部分设备上取不到值（返回 0），
   *  顶栏 / 底栏 / 弹窗的避让改由前端查询这里的真实高度。
   *
   *  单位必须换算：WindowInsets 给的是物理像素，而 WebView 里的 CSS px 是
   *  密度无关像素（dp），直接送回前端会整整大出一个屏幕密度倍数——3x 机上
   *  24dp 的状态栏被报成 72px，留出一条宽得离谱的白带。除以 density 换算
   *  成 dp 再返回，与 env() 的口径一致。 */
  @Command
  fun systemBarInsets(invoke: Invoke) {
    val rootInsets = activity.window?.decorView?.rootWindowInsets
    val insets = if (rootInsets != null) {
      WindowInsetsCompat.toWindowInsetsCompat(rootInsets)
    } else {
      WindowInsetsCompat.CONSUMED
    }
    val density = activity.resources.displayMetrics.density
    invoke.resolve(
      JSObject()
        .put(
          "top",
          (insets.getInsets(WindowInsetsCompat.Type.statusBars()).top / density).roundToInt(),
        )
        .put(
          "bottom",
          (insets.getInsets(WindowInsetsCompat.Type.navigationBars()).bottom / density).roundToInt(),
        )
    )
  }

  /** 设置状态栏图标配色：light = 深色图标（白色主题），false = 浅色图标（黑色主题）。
   *
   *  状态栏图标的深浅由系统决定，Web 侧没有任何 CSS 能改，只能经
   *  WindowInsetsControllerCompat 在原生侧切换。前端在应用界面风格时调用
   *  （见 src/theme.ts 的 applyTheme）。
   *
   *  同时把选择记进 StatusBarPrefs：MainActivity 冷启动时先读它作为初值，
   *  黑色主题的用户就不必每次打开都先闪一下深色图标（见 MainActivity.onCreate）。
   *  这份偏好是 MainActivity 与本插件之间唯一的共享契约，见 StatusBarPrefs。 */
  @Command
  fun setStatusBarStyle(invoke: Invoke) {
    val args = try {
      invoke.parseArgs(StatusBarArgs::class.java)
    } catch (e: Throwable) {
      invoke.reject("状态栏参数解析失败")
      return
    }
    StatusBarPrefs.setLightStatusBars(activity, args.light)
    applyLightStatusBars(args.light)
    invoke.resolve()
  }

  /** 切换状态栏图标深浅；窗口装饰只能在主线程动，命令线程不确定，包一层 */
  private fun applyLightStatusBars(light: Boolean) {
    activity.runOnUiThread {
      WindowInsetsControllerCompat(activity.window, activity.window.decorView)
        .isAppearanceLightStatusBars = light
    }
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
      saveViaMediaStore(invoke, src, fileName, ThrottledProgress(args.channel))
    } else {
      startCreateDocument(invoke, src, fileName)
    }
  }

  /** MediaStore 直写下载目录（API 29+）。IS_PENDING 先置 1，复制完成再归零，
   *  半截文件不会暴露给其他应用；中途失败把这条记录删掉，不留垃圾。 */
  @RequiresApi(Build.VERSION_CODES.Q)
  private fun saveViaMediaStore(
    invoke: Invoke,
    src: File,
    fileName: String,
    progress: ThrottledProgress,
  ) {
    activity.runBackground(invoke, "写入下载目录失败") {
      val resolver = activity.contentResolver
      val name = allocateDownloadsName(
        exists = { candidate ->
          resolver.query(
            MediaStore.Downloads.EXTERNAL_CONTENT_URI,
            arrayOf(MediaStore.Downloads._ID),
            "${MediaStore.Downloads.DISPLAY_NAME} = ?",
            arrayOf(candidate),
            null
          )?.use { it.moveToFirst() } ?: false
        },
        fileName = fileName,
      )
      val values = ContentValues().apply {
        put(MediaStore.Downloads.DISPLAY_NAME, name)
        put(MediaStore.Downloads.MIME_TYPE, "application/zip")
        put(MediaStore.Downloads.IS_PENDING, 1)
      }
      val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
        ?: throw IllegalStateException("无法在下载目录创建文件")
      try {
        resolver.openOutputStream(uri)?.use { out ->
          copyFileWithProgress(src, out, progress)
        } ?: throw IllegalStateException("无法写入下载目录")
        val done = ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }
        resolver.update(uri, done, null, null)
        activity.resolveOnUi(invoke) { JSObject().put("name", name) }
      } catch (e: Throwable) {
        runCatching { resolver.delete(uri, null, null) }
        throw e
      }
    }
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
    val progress = ThrottledProgress(args.channel)
    activity.runBackground(invoke, "写入所选位置失败") {
      if (!src.isFile) throw IllegalStateException("找不到待导出的文件")
      val name = sanitizeName(
        activity.contentResolver.queryDisplayName(uri) ?: args.fileName ?: "漫画.zip"
      )
      activity.contentResolver.openOutputStream(uri)?.use { out ->
        copyFileWithProgress(src, out, progress)
      } ?: throw IllegalStateException("无法写入所选位置")
      activity.resolveOnUi(invoke) { JSObject().put("name", name) }
    }
  }

  /** 把文件复制进输出流，按 64KB 块回调进度（节流在 ThrottledProgress 里做）。
   *  与 copyUriToFile 同样不用 InputStream.copyTo：单个大 zip 会让进度条
   *  长时间停在 0 再一步跳到 100%。 */
  private fun copyFileWithProgress(src: File, out: OutputStream, progress: ThrottledProgress) {
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
          progress.send {
            put("phase", "save")
            put("doneBytes", done)
            put("totalBytes", total)
          }
        }
      }
    }
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
}
