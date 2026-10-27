package com.conceit.mangareader

/**
 * 清洗文档提供者返回的名称。
 *
 * 路径分隔符、NUL 与前导点都可能让文件被写到缓存目录之外（如 "../../xxx"），
 * 统一替换掉。SAF 的显示名来自任意第三方应用，不能信任——这是导入链路上
 * 唯一防目录穿越的关卡，也是最适合做单测的一个纯函数。
 */
internal fun sanitizeName(raw: String): String {
  val cleaned = raw
    .replace('/', '_')
    .replace('\\', '_')
    .replace('\u0000', '_')
    .trim()
    .trimStart('.')
  return cleaned.ifEmpty { "unnamed" }
}
