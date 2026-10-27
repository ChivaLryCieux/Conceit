// 阅读器缩放：比例范围与夹取。底栏按钮、Ctrl+滚轮、双指捏合三条入口共用，
// 边界只在这里定义一次。

export const ZOOM_MIN = 0.5;
export const ZOOM_MAX = 3;

/** 夹到 [ZOOM_MIN, ZOOM_MAX]，并按 0.01 取整——百分比显示才稳定 */
export function clampZoom(z: number): number {
  return Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Math.round(z * 100) / 100));
}

/** 是否处于放大状态（1 = 宽度铺满，即原始大小）。
 *  留 0.001 容差：捏合 / 滚轮结束时比例常落在 1.0000001 之类，
 *  按 > 1 判断会误判成"已放大"，横向平移白接一层 */
export function isZoomed(z: number): boolean {
  return z > 1.001;
}
