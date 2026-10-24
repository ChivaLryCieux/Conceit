import { useCallback, useState } from "react";

/** UI 风格：白色（默认） / 黑色 */
export type Theme = "white" | "black";

const STORAGE_KEY = "conceit-theme";

/** 各风格下系统栏的建议颜色（同步到 index.html 的 theme-color） */
const THEME_COLORS: Record<Theme, string> = {
  white: "#f6f6f9",
  black: "#14151a",
};

function applyTheme(theme: Theme): void {
  document.documentElement.dataset.theme = theme;
  document
    .querySelector('meta[name="theme-color"]')
    ?.setAttribute("content", THEME_COLORS[theme]);
}

function loadTheme(): Theme {
  try {
    return localStorage.getItem(STORAGE_KEY) === "black" ? "black" : "white";
  } catch {
    return "white";
  }
}

/** 启动时恢复风格。必须在 React 挂载、首帧绘制之前调用，
 *  否则会闪一下另一风格再切换。 */
export function initTheme(): void {
  applyTheme(loadTheme());
}

/** 设置菜单用的风格状态：切换立即生效（html[data-theme]）并记住选择 */
export function useTheme(): [Theme, (theme: Theme) => void] {
  const [theme, setThemeState] = useState<Theme>(loadTheme);
  const setTheme = useCallback((next: Theme) => {
    setThemeState(next);
    applyTheme(next);
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      // 存不进（隐私模式等）只是记不住，切换本身不受影响
    }
  }, []);
  return [theme, setTheme];
}
