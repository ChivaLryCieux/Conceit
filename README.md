# Conceit · 幻想阅读器

**Conceit**（中文名：**幻想阅读器**）是一个使用 [Tauri 2](https://v2.tauri.app/) + React + TypeScript 构建的本地漫画阅读器：书架式陈列、自由排序、竖向连续阅读，所有漫画都会复制进应用自维护的书库目录，源文件之后移动或删除都不受影响。

## 功能特性

- **书架主页**：漫画与文件夹以封面网格陈列，像书架一样一目了然；面包屑导航进入 / 返回文件夹
- **自由排序**：拖拽卡片调整顺序（带前 / 后插入线指示），拖到文件夹卡片即移入该文件夹，拖到面包屑可移动到任意层级；排序写入 `index.json`，重启后保持
- **条目设置**：每个漫画 / 文件夹名称右侧设有设置按钮，可修改名称、开关封面展示（默认纯色封面）、选择漫画任意一页作为封面
- **连续阅读**：打开漫画后，文件夹中的图片竖向拼接，滚轮 / 方向键 / 进度滑块上下滑动连续阅读；默认宽度填充整个屏幕，可缩放（50% – 300%）
- **阅读浮窗**：点击画面显隐唯一的顶部工具栏——退出阅读、漫画名称、缩放控制、进度滑块、页码与百分比；工具栏是浮层，隐藏时漫画完全铺满竖直方向，顶部不留黑边
- **多种缩放方式**：顶栏内点击 `− 百分比 +` 逐档缩放、点百分比复位；按住 **Ctrl 滚动滚轮**（触控板捏合同效）即可平滑缩放，缩放时自动显出工具栏显示当前比例
- **阅读进度**：自动记录并恢复每部漫画的阅读位置，书架卡片上显示进度条与百分比
- **本地导入**（三种方式）：
  - 「导入文件夹」：选择一个或多个本机漫画目录，递归复制其中所有图片
  - 「导入 zip」：导入压缩包，自动解压
  - **拖拽导入**：把文件夹 / zip 直接拖到应用窗口（导入到当前所在文件夹）
- **书库管理**：新建文件夹（应用内弹窗）、重命名、删除（文件夹递归删除其中内容）、移动到根目录
- **移动端适配**：响应式布局（手机 / 平板 / 小屏手机）、安全区适配、触屏手势（长按打开操作菜单、长按拖动排序）、底部弹出式设置面板、Pinch 缩放
- **纯 SVG 图标**：界面不使用 emoji，全部为内联 SVG 图标

## 快速开始

环境要求：Node.js 18+、pnpm、Rust stable。

```bash
pnpm install       # 安装依赖
pnpm tauri dev     # 开发模式（热更新）
pnpm tauri build   # 打包发布版
```

> **dev 模式说明**：请通过 `pnpm dev` / `pnpm tauri dev` 启动（已内置修复逻辑）。
> 某些环境下 pnpm store 里的 `esbuild.exe` 被设置了低完整性级别标签（Mandatory
> Label\Low），会导致 esbuild 服务进程无法写文件、vite 依赖预打包失败、页面白屏。
> `scripts/dev.mjs` 会自动复制一份干净的 esbuild 并用 `ESBUILD_BINARY_PATH` 指向它，
> 正常机器上同样无害。直接运行 `vite` 或 `npx vite` 不会经过该修复。

**阅读器行为验证**（可选）：`scripts/verify-reader.cjs` 通过 WebView2 远程调试（CDP）
驱动真实 UI，自动检查浮层工具栏、点击显隐、Ctrl+滚轮缩放等行为并输出通过情况。
先以 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 pnpm tauri dev`
启动应用，再另开终端执行 `node scripts/verify-reader.cjs`。

**条目设置验证**（可选）：`scripts/verify-cover.cjs` 通过真实点击走一遍
「打开设置 → 勾选展示封面 → 保存 → 书架渲染封面图 → 重开设置状态仍在」的闭环：
`node scripts/verify-cover.cjs`（同样需要 `--remote-debugging-port=9222` 的 dev 实例）。

**移动端适配验证**（可选）：`scripts/verify-mobile.cjs` 在同一 CDP 连接上模拟
小屏手机 / 手机 / 平板三种视口与触摸事件，检查无横向溢出、设置按钮常显、
长按菜单、长按拖动排序、顶栏两行排布、底部弹窗等，并输出通过情况：
`node scripts/verify-mobile.cjs`（需 `--remote-debugging-port=9222` 的 dev 实例正在运行）。

发布产物位于 `src-tauri/target/release/bundle/`（Windows 下为 `conceit.exe`、`.msi` 安装包与 `-setup.exe` 安装程序）。

## 移动端适配

界面已按移动端优先做好适配，桌面 / 平板 / 手机共用同一套代码：

| 维度 | 适配内容 |
| --- | --- |
| 布局 | 断点 1024 / 640 / 380px；书架卡片网格自动 2–5 列；头部品牌与操作按钮在窄屏分行，面包屑可横向滚动 |
| 触屏 | 设置按钮常显（桌面为 hover 显现）；去掉 hover 粘滞效果，改用按压缩放反馈；按钮点击目标加大 |
| 手势 | 长按卡片打开操作菜单（等价于桌面右键）；长按后拖动即可排序，浮动卡片跟随手指，落到文件夹上为移入；捏合 / Ctrl+滚轮缩放漫画 |
| 阅读器 | 顶部工具栏在窄屏自动两行排布；漫画始终铺满屏幕；工具栏显隐不影响内容位置 |
| 弹窗 | 手机上模态框变为底部弹出面板，内部滚动，避让 Home 指示条 |
| 安全区 | `env(safe-area-inset-*)` 已接入头部、工具栏、提示条、Toast 与弹窗，适配刘海 / 小黑条 |
| 视口 | `viewport-fit=cover` + 禁用页面缩放（缩放由应用内部控制） |

书库路径使用 Tauri 跨平台的 `app_data_dir()`，Android / iOS 上会自动落到应用私有目录，无需改动。

### 发布移动版

图标资源（Android 自适应图标、iOS AppIcon）已随仓库生成在 `src-tauri/icons/android` 与 `src-tauri/icons/ios`。在此基础上：

**Android**（需要 JDK 17+ 与 Android SDK）：

```bash
pnpm tauri android init      # 首次：生成 src-tauri/gen/android 工程
pnpm tauri android dev       # 连接到设备/模拟器调试
pnpm tauri android build     # 产出 APK / AAB
```

**iOS**（需要 macOS + Xcode，在 Mac 上执行）：

```bash
pnpm tauri ios init
pnpm tauri ios dev
pnpm tauri ios build
```

> `tauri android init` 会校验 Java 环境（`JAVA_HOME` 或 Android Studio 自带 JBR），缺失时会中断——属预期行为，装好 JDK 后重试即可。
> 生成的 `gen/android`、`gen/apple` 工程已纳入版本管理（`.gitignore` 只忽略每次构建重新生成的 `gen/schemas/`）。

## 书库目录结构

书库位于 `<应用数据目录>/library`，Windows 下通常为
`%APPDATA%\com.conceit.mangareader\library`：

```
library/
├── index.json          # 书架索引：条目、父子关系、每个容器内的排序、阅读进度
└── manga/
    ├── m1/             # 每部漫画一个目录（导入时复制 / 解压生成）
    │   ├── 001.jpg
    │   ├── 002.jpg
    │   └── ...
    └── m2/
```

- `index.json` 是应用唯一需要维护的状态文件；删除它可以重置书架（已复制的图片文件不受影响）
- 支持的图片格式：jpg / jpeg / png / webp / gif / bmp / avif

## 技术架构

```
src/                         前端（React 18 + Vite 5 + TypeScript 严格模式）
├── App.tsx                  应用状态：书架 / 阅读器切换、文件夹栈、拖拽导入、Toast
├── api.ts                   对 Rust 命令的封装 + 书架条目排序
├── types.ts                 与 Rust 数据模型一一对应的类型（含 Toast）
├── styles/                  样式，按关注点拆分，由 index.css 统一引入
│   ├── base.css             变量、重置、应用外壳、按钮、通用动画、滚动条
│   ├── shelf.css            书架：品牌区、面包屑、卡片网格、条目卡片、右键菜单
│   ├── reader.css           阅读器：浮层顶栏、缩放控件、进度滑块、滚动区
│   ├── overlay.css          忙碌遮罩、Toast、模态弹窗、表单、封面选页
│   └── mobile.css           移动端 / 触屏适配（断点与 hover:none）
├── components/
│   ├── shelf/               书架功能域
│   │   ├── Shelf.tsx        编排：状态、弹窗、网格渲染
│   │   ├── ShelfHeader.tsx  品牌区 + 面包屑 + 操作按钮
│   │   ├── Card.tsx         漫画 / 文件夹卡片（封面 / 纯色封面、进度条、设置按钮）
│   │   ├── useShelfMutations.ts  导入 / 新建 / 删除 / 移动
│   │   └── useItemSorting.ts     鼠标拖放 + 触屏长按排序（共用落点计算）
│   ├── reader/              阅读器功能域
│   │   ├── Reader.tsx       图片加载、进度恢复与保存、缩放、键盘操作
│   │   └── ReaderTopbar.tsx 唯一顶栏（退出 / 标题 / 缩放 / 进度滑块 / 计数）
│   ├── ItemSettings.tsx     条目设置（名称 / 封面开关 / 封面页选择）
│   └── ui/                  通用组件：Modal、InputDialog、ContextMenu、icons

src-tauri/                   后端（Rust + Tauri 2）
├── src/library.rs           书库模型：索引持久化、自然排序、目录复制、zip 解压、漫画数量统计
├── src/commands.rs          Tauri 命令层（快照 / 增删改 / 排序 / 进度 / 导入）
└── tauri.conf.json          桌面配置（asset 协议加载本地图片）
```

### 实现要点

- **自维护书库**：导入即把图片复制进 `library/manga/<id>/`，书库与源目录完全独立
- **asset 协议**：前端通过 `convertFileSrc` 直接加载书库内的本地图片，无需额外读取权限
- **自然排序**：`page2.jpg` 排在 `page10.jpg` 之前，符合漫画页码习惯
- **封面策略**：默认纯色封面（按条目 id 派生稳定色值），可在设置中开启封面图并任选一页作为封面
- **进度恢复**：退出或滚动时节流写入 `index.json`，下次打开按比例恢复滚动位置
- **文件夹即容器**：每个容器（根目录或文件夹）独立维护排序，移动条目只改变父子关系
- **兼容旧索引**：`show_cover` 等新字段均带 serde 默认值，旧 `index.json` 可直接读取

## 已知限制

- 拖拽排序基于 HTML5 Drag & Drop，触屏设备暂不支持
- 超大漫画（数百页）会一次性加载全部图片，内存占用较高

## 命名

| | |
|---|---|
| 包名 / 产品名 | **Conceit** |
| 中文名 | **幻想阅读器** |
| 应用标识 | `com.conceit.mangareader` |
