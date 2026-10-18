# Conceit · 漫画阅读器

一个使用 **Tauri 2 + React + TypeScript** 构建的本地漫画阅读器。

- 书架式主页：漫画与文件夹平铺陈列，支持拖拽自由排序
- 沉浸式阅读：将漫画文件夹中的图片竖向拼接，上下滑动连续阅读
- 点击画面呼出浮窗工具栏（返回、进度、退出等）
- 从本机目录（或 zip）导入漫画，自动复制到应用自维护的书库目录，源文件删除也不受影响
- 自动记录每部漫画的阅读进度

## 开发

```bash
pnpm install
pnpm tauri dev
```

## 构建

```bash
pnpm tauri build
```

## 目录结构

```
src/            前端（React + Vite）
src-tauri/      后端（Rust + Tauri 2）
```

详细说明见 [README](#)（完善中）。
