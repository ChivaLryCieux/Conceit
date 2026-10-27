package com.conceit.mangareader

import android.content.ContentResolver
import android.net.Uri
import android.provider.OpenableColumns
import app.tauri.Logger
import java.io.File
import java.io.FileOutputStream

/**
 * content:// URI 的读取与落盘原语。
 *
 * 只依赖 ContentResolver，不碰 Activity 生命周期——因此可以在 JVM 上直接
 * 单测（伪造 resolver 即可），这是它们从 ConceitPlugin 搬出来的原因。
 */

/** 查询 content:// URI 的显示名；提供者不给时返回 null */
internal fun ContentResolver.queryDisplayName(uri: Uri): String? {
  val projection = arrayOf(OpenableColumns.DISPLAY_NAME)
  query(uri, projection, null, null, null)?.use { cursor ->
    if (cursor.moveToFirst()) {
      val idx = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
      if (idx >= 0) return cursor.getString(idx)
    }
  }
  return null
}

/** 查询 content:// URI 的文件大小；提供者不给时返回 null */
internal fun ContentResolver.querySize(uri: Uri): Long? {
  val projection = arrayOf(OpenableColumns.SIZE)
  query(uri, projection, null, null, null)?.use { cursor ->
    if (cursor.moveToFirst()) {
      val idx = cursor.getColumnIndex(OpenableColumns.SIZE)
      if (idx >= 0 && !cursor.isNull(idx)) return cursor.getLong(idx)
    }
  }
  return null
}

/**
 * 把 content:// URI 的内容写成一个真实文件。
 *
 * 按 64KB 块回调本次搬运的字节数（进度用）——不用 InputStream.copyTo：
 * 那个重载不给中间进度，单个大文件会让进度条长时间停在 0 再一步跳到 100%。
 * 打开失败（提供者拒绝、文件已消失）返回 false，调用方跳过该文件继续。
 */
internal fun ContentResolver.copyToFile(
  uri: Uri,
  out: File,
  onBytes: (Long) -> Unit = {},
): Boolean {
  val stream = openInputStream(uri)
  if (stream == null) {
    Logger.error("无法打开所选文件: $uri")
    return false
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
  return true
}

/**
 * 把 content:// URI 复制成 dir 下的真实文件，返回其绝对路径（打开失败返回 null）。
 *
 * 保留原始名称，导入后的漫画标题才是有意义的文件夹名 / 压缩包名；同名文件
 * 自动加序号（见 allocateUniqueFile）。ensureZipExtension：少数提供者查不到
 * 显示名，回退值可能形如 "msf:42"（含点但不是 .zip）；导入端按扩展名判断
 * zip，这里统一保证后缀正确。
 */
internal fun ContentResolver.copyUriToFile(
  uri: Uri,
  dir: File,
  ensureZipExtension: Boolean,
  onBytes: (Long) -> Unit = {},
): String? {
  var name = sanitizeName(
    queryDisplayName(uri) ?: uri.lastPathSegment ?: "file-${System.currentTimeMillis()}"
  )
  if (ensureZipExtension && !name.endsWith(".zip", ignoreCase = true)) name = "$name.zip"
  val out = allocateUniqueFile(dir, name)
  if (!copyToFile(uri, out, onBytes)) return null
  return out.absolutePath
}
