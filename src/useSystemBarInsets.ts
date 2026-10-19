import { useEffect } from "react";
import { systemBarInsets } from "./api";

/** 把系统栏高度同步成 CSS 变量 --status-bar-h / --nav-bar-h。
 *
 *  安卓 WebView 的 env(safe-area-inset-*) 在部分设备上取不到值（返回 0），
 *  顶栏 / 底栏 / 弹窗的避让不能只依赖它。这里在启动时、以及每次窗口尺寸
 *  变化（旋屏、折叠屏形态切换——Activity 声明了 configChanges，页面不重载）
 *  后向原生桥查询一次真实高度，覆写 :root 里的 env() 默认值。
 *  桌面端命令不存在，静默跳过，CSS 变量保持 env() 回退。 */
export function useSystemBarInsets() {
  useEffect(() => {
    let alive = true;
    const sync = async () => {
      try {
        const { top, bottom } = await systemBarInsets();
        if (!alive) return;
        const root = document.documentElement;
        root.style.setProperty("--status-bar-h", `${top}px`);
        root.style.setProperty("--nav-bar-h", `${bottom}px`);
      } catch {
        // 桌面端没有这条命令：忽略，沿用 :root 的 env() 回退值
      }
    };
    void sync();
    window.addEventListener("resize", sync);
    return () => {
      alive = false;
      window.removeEventListener("resize", sync);
    };
  }, []);
}
