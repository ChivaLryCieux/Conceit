package com.conceit.mangareader

import android.content.Context
import java.io.File

/**
 * 导入桥在 cacheDir 下占用的目录。
 *
 * 目录名只在这里出现一次：新增一个选择器只需加一个枚举项，清理自动覆盖。
 * 原先三个名字散落在各命令里，漏改一处就是缓存残留——而缓存里有上一次
 * 导入的残留时，一部漫画会被导入两份（见 ConceitPlugin.directoryPicked
 * 里那条「本次 N 个 / 磁盘共 M 个」的诊断日志，正是为这个问题加的）。
 */
internal enum class ImportCache(val dirName: String) {
  /** zip / 图片多选：复制进缓存供 Rust 读取 */
  PICKED_IMPORT("picked-import"),

  /** SAF 目录树：整棵树复制进缓存 */
  TREE_IMPORT("tree-import"),

  /** 设置页「增加图片」 */
  PICKED_IMAGES("picked-images");

  /** 对应的缓存目录 */
  fun dir(context: Context): File = File(context.cacheDir, dirName)

  /** 取出缓存目录；wipe = true 时先清空再重建（每次导入都是全新的一份） */
  fun prepare(context: Context, wipe: Boolean = false): File {
    val dir = dir(context)
    if (wipe) dir.deleteRecursively()
    dir.mkdirs()
    return dir
  }
}
