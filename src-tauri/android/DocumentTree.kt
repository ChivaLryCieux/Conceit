package com.conceit.mangareader

import android.content.ContentResolver
import android.net.Uri
import android.provider.DocumentsContract
import java.io.File

/**
 * SAF 目录树的遍历与整树操作。
 *
 * 用显式栈而非递归：深层目录树会把线程栈爆掉，而 StackOverflowError 不是
 * Exception、catch 不住，未捕获的 Error 会直接让进程闪退。
 *
 * 只负责「枚举」：每一项以相对根的路径给出，本地怎么落（建目录复制、只累加
 * 大小、只取根显示名）由调用方决定。原先 walkTree 把枚举与落点揉在一起，
 * 纯统计大小时要向 dest 传一个假的 File("/") 才能复用。
 */

/** 目录树中的一个条目 */
internal data class TreeEntry(
  /** 文档 URI（文件内容或子目录列表都从它取） */
  val uri: Uri,
  /** 相对根的路径，如 "第1话/001.jpg"；目录条目不含结尾分隔符 */
  val relativePath: String,
  /** 文件大小；目录条目与「提供者不给大小」时为 null */
  val size: Long?,
  val isDir: Boolean,
)

/**
 * 深度优先遍历一棵 SAF 目录树。
 *
 * inline：treeTotalSize 靠调用方 lambda 里的非局部 return 提前收手
 * （任一文件大小未知就整体放弃预估）。visitDir 对每个子目录回调，
 * visitFile 对每个文件回调。
 */
internal inline fun ContentResolver.walkTree(
  treeUri: Uri,
  visitDir: (TreeEntry) -> Unit = {},
  visitFile: (TreeEntry) -> Unit,
) {
  // 显式栈而非递归：深层目录树的 StackOverflowError 不是 Exception、catch 不住
  val projection = arrayOf(
    DocumentsContract.Document.COLUMN_DOCUMENT_ID,
    DocumentsContract.Document.COLUMN_MIME_TYPE,
    DocumentsContract.Document.COLUMN_DISPLAY_NAME,
    DocumentsContract.Document.COLUMN_SIZE,
  )
  // 栈里是（子文档列表 URI，该层相对根的路径前缀）
  val stack = ArrayDeque<Pair<Uri, String>>()
  stack.addLast(
    DocumentsContract.buildChildDocumentsUriUsingTree(
      treeUri,
      DocumentsContract.getTreeDocumentId(treeUri)
    ) to ""
  )
  while (stack.isNotEmpty()) {
    val (childrenUri, prefix) = stack.removeLast()
    query(childrenUri, projection, null, null, null)?.use { cursor ->
      while (cursor.moveToNext()) {
        val docId = cursor.getString(0) ?: continue
        val mime = cursor.getString(1)
        val name = sanitizeName(cursor.getString(2) ?: docId)
        val relativePath = if (prefix.isEmpty()) name else "$prefix/$name"
        val docUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, docId)
        if (mime == DocumentsContract.Document.MIME_TYPE_DIR) {
          visitDir(TreeEntry(docUri, relativePath, null, isDir = true))
          stack.addLast(
            DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, docId) to relativePath
          )
        } else {
          val size = if (cursor.isNull(3)) null else cursor.getLong(3)
          visitFile(TreeEntry(docUri, relativePath, size, isDir = false))
        }
      }
    }
  }
}

/** 整棵树复制进 dest：保留目录结构（含空目录）与原始文件名 */
internal fun ContentResolver.copyTree(treeUri: Uri, dest: File, onFile: (Long) -> Unit = {}) {
  walkTree(
    treeUri,
    visitDir = { File(dest, it.relativePath).mkdirs() },
    visitFile = { entry ->
      val slash = entry.relativePath.lastIndexOf('/')
      val dir = if (slash < 0) dest else File(dest, entry.relativePath.substring(0, slash))
      copyUriToFile(entry.uri, dir, ensureZipExtension = false) { n -> onFile(n) }
    },
  )
}

/** 统计目录树全部文件大小；任一文件大小未知时返回 null（放弃预估、直接放行） */
internal fun ContentResolver.treeTotalSize(treeUri: Uri): Long? {
  var total = 0L
  walkTree(treeUri, visitFile = { entry ->
    entry.size ?: return null
    total += entry.size
  })
  return total
}

/** 所选目录的显示名（作为导入后的漫画标题） */
internal fun ContentResolver.treeDisplayName(treeUri: Uri): String {
  val docId = DocumentsContract.getTreeDocumentId(treeUri)
  val docUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, docId)
  return sanitizeName(queryDisplayName(docUri) ?: docId.substringAfterLast(':'))
}
