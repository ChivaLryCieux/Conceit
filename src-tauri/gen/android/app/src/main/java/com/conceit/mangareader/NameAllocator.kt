package com.conceit.mangareader

import java.io.File

/**
 * 同名冲突时的序号命名。
 *
 * 仓库里有两处「目标已存在就换个名字」：多选文件落缓存（stem-n.ext，从 2 起）
 * 与导出到下载目录（stem (n).ext，与桌面端 Rust unique_dest_path 同规则，从 1 起）。
 * 两条规则故意不同，但「拆词干 / 拆扩展名 / 拼序号」的骨架完全相同——
 * 只写一次，两边各自给格式与起始序号。
 */

/** 把文件名拆成「词干 + 扩展名」（无点号时扩展名为空） */
internal fun splitName(name: String): Pair<String, String> =
  name.substringBeforeLast('.', name) to name.substringAfterLast('.', "")

/** 缓存目录里的同名文件：name、name-2、name-3…（一次多选时互相不覆盖） */
internal fun allocateUniqueFile(dir: File, name: String): File {
  var candidate = File(dir, name)
  if (!candidate.exists()) return candidate
  val (stem, ext) = splitName(name)
  var n = 2
  while (candidate.exists()) {
    candidate = if (ext.isEmpty()) File(dir, "$stem-$n") else File(dir, "$stem-$n.$ext")
    n += 1
  }
  return candidate
}

/**
 * 下载目录里的同名文件：name、name (1)、name (2)…；999 个之后退时间戳。
 *
 * 导出是读者自己的资产，不覆盖上一次的结果——与桌面端 Rust 的
 * unique_dest_path 保持同一规则。exists 由调用方给出（MediaStore 查询），
 * 这里只负责拼名，便于单测。
 */
internal fun allocateDownloadsName(exists: (String) -> Boolean, fileName: String): String {
  if (!exists(fileName)) return fileName
  val (stem, ext) = splitName(fileName)
  for (n in 1..999) {
    val candidate = if (ext.isEmpty()) "$stem ($n)" else "$stem ($n).$ext"
    if (!exists(candidate)) return candidate
  }
  val stamp = System.currentTimeMillis()
  return if (ext.isEmpty()) "$stem ($stamp)" else "$stem ($stamp).$ext"
}
