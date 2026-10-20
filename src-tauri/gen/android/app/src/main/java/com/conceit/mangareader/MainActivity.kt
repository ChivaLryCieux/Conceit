package com.conceit.mangareader

import android.os.Bundle
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge
import androidx.core.view.WindowInsetsControllerCompat

/**
 * Conceit 的主 Activity。
 *
 * 相对 tauri 生成的默认版本，多了两处针对性改动：
 * 1. **拦截系统返回**：TauriActivity 关闭了 wry 的默认返回处理
 *    （handleBackNavigation = false），不拦截时系统会直接结束
 *    Activity——也就是在阅读器里划边缘返回也会退出应用。这里把「返回」
 *    改判给前端：阅读器→回书架、文件夹→上一层、根目录→退出。
 * 2. **系统栏图标用白色**：界面是深色的，而主题是 DayNight，浅色系统下
 *    系统会给深色图标，压在深色内容上看不清。
 *
 * 注意：本文件同时是 gen/android 下同名文件的规范副本，
 * scripts/build-android.mjs 每次构建前会把 src-tauri/android/ 下的 Kotlin
 * 源码同步过去，`tauri android init` 重建工程也不会丢失这里的改动。
 */
class MainActivity : TauriActivity() {
  /** wry 创建好 WebView 后经 onWebViewCreate 交出引用，返回手势转发时使用 */
  private var webView: WebView? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)

    // 深色界面：状态栏与底部导航栏的图标 / 文字都用浅色。
    // enableEdgeToEdge 会按 DayNight 主题设置，这里显式压回浅色图标
    WindowInsetsControllerCompat(window, window.decorView).apply {
      isAppearanceLightStatusBars = false
      isAppearanceLightNavigationBars = false
    }

    // 硬件返回键与边缘滑动手势都会走 OnBackPressedDispatcher。
    // 注册在 super.onCreate() 之后：wry 因 handleBackNavigation = false 不会
    // 注册自己的回调，这里便是 dispatcher 中唯一启用的回调，拿到全部返回事件。
    onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
      override fun handleOnBackPressed() {
        // 转发成 DOM 事件，前端按当前状态决定去向（见 src/App.tsx）。
        // WebView 尚未就绪时事件自然丢失，此时按默认退出处理也可接受。
        webView?.evaluateJavascript(
          "window.dispatchEvent(new CustomEvent('conceit:android-back'))",
          null
        )
      }
    })
  }

  /** wry 在 WebView 就绪后调用（见 WryActivity.setWebView），留存引用 */
  override fun onWebViewCreate(webView: WebView) {
    this.webView = webView
  }
}
