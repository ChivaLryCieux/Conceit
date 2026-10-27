package com.conceit.mangareader

import android.content.Context

/**
 * 状态栏图标配色的持久化。
 *
 * 这是 MainActivity 与 ConceitPlugin 之间唯一的共享契约：冷启动时
 * MainActivity 先读它作为初值（WebView 首帧画出来之前系统栏已经在显示，
 * 选错会闪一下另一种配色），之后由前端经 ConceitPlugin.setStatusBarStyle
 * 切换时写入。原先两端直接共用插件类的常量——Activity 反向依赖 Plugin 类；
 * 收成一个对象后，两端都只依赖这份契约本身。
 */
internal object StatusBarPrefs {
  /** 界面状态的持久化文件 */
  const val NAME = "conceit-ui"

  /** 状态栏图标是否用深色：true = 白色主题，false = 黑色主题 */
  const val KEY_LIGHT_STATUS_BARS = "light_status_bars"

  /** 读取上次的选择；没有记录（首次安装）按默认的白色主题给深色图标 */
  fun lightStatusBars(context: Context): Boolean =
    context.getSharedPreferences(NAME, Context.MODE_PRIVATE)
      .getBoolean(KEY_LIGHT_STATUS_BARS, true)

  /** 记录选择（前端切换界面风格时由插件写入） */
  fun setLightStatusBars(context: Context, light: Boolean) {
    context.getSharedPreferences(NAME, Context.MODE_PRIVATE)
      .edit()
      .putBoolean(KEY_LIGHT_STATUS_BARS, light)
      .apply()
  }
}
