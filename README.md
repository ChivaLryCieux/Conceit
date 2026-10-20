# Conceit · 幻想阅读器

**Conceit**（中文名：**幻想阅读器**）是一个使用 [Tauri 2](https://v2.tauri.app/) + React + TypeScript 构建的本地漫画阅读器：书架式陈列、自由排序、竖向连续阅读，所有漫画都会复制进应用自维护的书库目录，源文件之后移动或删除都不受影响。

## 功能特性

- **书架主页**：漫画与文件夹以封面网格陈列，像书架一样一目了然；面包屑导航进入 / 返回文件夹；导入 / 新建 / 导航功能栏固定底部（平板与手机上拇指更好够到，顶部也不被状态栏占掉一块）
- **自由排序**：拖拽卡片调整顺序（带前 / 后插入线指示），拖到文件夹卡片即移入该文件夹，拖到面包屑可移动到任意层级；排序写入 `index.json`，重启后保持
- **条目设置**：每个漫画 / 文件夹名称右侧设有设置按钮，可修改名称、开关封面展示（默认纯色封面）、选择漫画任意一页作为封面；漫画还能**增加图片**（多选，追加到末尾）、**删除图片**（✕ 标记后统一确认）与**拖动调整图片顺序**
- **连续阅读**：打开漫画后，文件夹中的图片竖向拼接，滚轮 / 方向键 / 进度滑块上下滑动连续阅读；默认宽度填充整个屏幕，可缩放（50% – 300%）
- **阅读浮窗**：点击画面显隐唯一的底部工具栏——退出阅读、漫画名称、缩放控制、进度滑块、页码与百分比；工具栏是浮层，隐藏时漫画完全铺满竖直方向，顶部不留黑边。放在底部是为了避开手机状态栏、减少误触
- **多种缩放方式**：底栏内点击 `− 百分比 +` 逐档缩放、点百分比复位；按住 **Ctrl 滚动滚轮**（触控板捏合同效）即可平滑缩放，缩放时自动显出工具栏显示当前比例
- **阅读进度**：自动记录并恢复每部漫画的阅读位置，书架卡片上显示进度条与百分比
- **本地导入**（三种方式）：
  - 「导入文件夹」：选择一个或多个本机漫画目录，递归复制其中所有图片
  - 「导入 zip」：导入压缩包，自动解压
  - **拖拽导入**：把文件夹 / zip 直接拖到应用窗口（导入到当前所在文件夹）
  - Android 上改用系统文档选择器（SAF）完成目录 / zip 选择，见下文「Android 导入桥」
- **书库管理**：新建文件夹（应用内弹窗）、重命名、删除（文件夹递归删除其中内容）、移动到根目录
- **封面缩略图**：书架封面使用导入时预生成的 480px 缩略图（原图动辄数 MB，避免 WebView 为一个小卡片解码整张大图）；阅读仍加载原图
- **移动端适配**：响应式布局（手机 / 平板 / 小屏手机）、安全区适配、触屏手势（长按打开操作菜单、长按拖动排序）、底部弹出式设置面板、Pinch 缩放
- **系统返回**（安卓）：硬件返回键与边缘滑动手势统一拦截——阅读器中返回到书架、书架内逐级返回文件夹、只有根目录才退出应用；弹窗与操作菜单优先响应返回
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
「打开设置 → 勾选展示封面 → 保存 → 书架渲染封面图 → 重开设置状态仍在 → 封面加载的是缩略图」的闭环：
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
| 布局 | 断点 1024 / 640 / 380px；书架卡片网格自动 2–5 列；功能栏固定底部，窄屏分两行（品牌 + 面包屑 / 操作按钮），面包屑可横向滚动 |
| 触屏 | 设置按钮常显（桌面为 hover 显现）；去掉 hover 粘滞效果，改用按压缩放反馈；按钮点击目标加大 |
| 手势 | 长按卡片打开操作菜单（等价于桌面右键）；长按后拖动即可排序，浮动卡片跟随手指，落到文件夹上为移入；捏合 / Ctrl+滚轮缩放漫画 |
| 系统返回 | 硬件返回键与边缘滑动手势由 `MainActivity` 拦截并转交前端：阅读器→书架、文件夹逐级返回、根目录才退出；模态框 / 操作菜单优先消费 |
| 阅读器 | 底部工具栏在窄屏自动两行排布；漫画始终铺满屏幕；工具栏显隐不影响内容位置 |
| 弹窗 | 手机上模态框变为底部弹出面板，内部滚动，避让 Home 指示条 |
| 条目设置 | 缩略图网格支持点选封面、✕ 标记删除（可撤销）、鼠标拖放 / 触屏长按拖动排序；手机上删除靶子加大到 24px |
| 安全区 | 状态栏 / 导航栏高度经原生桥同步为 CSS 变量（安卓 WebView 的 `env(safe-area-inset-*)` 部分设备取不到值），底部功能栏、工具栏、提示条、Toast 与弹窗均已避让 |
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

### 打包 Android APK（Windows 免管理员）

`pnpm tauri android build` 在 Windows 上有一个坎：它要把 Rust 编出的 `.so` 软链到
`gen/android/app/src/main/jniLibs/`，而创建符号链接需要「开发者模式」或管理员权限。
没有这些权限时命令会在 Rust 编译成功之后、链接那一步中断。

本仓库改用 `scripts/build-android.mjs` 完成同样的搬运（普通文件复制），再直接调用 Gradle，
产物与官方流程一致。签名配置也已配好，产出的就是可直接安装的 release APK。

> **两个必须复刻的细节**（脚本已处理，手工跑 cargo 时容易漏）：
> 1. `cargo build` 必须带 `--features tauri/custom-protocol`。这个特性决定 tauri 走
>    「内嵌 assets 的自定义协议」还是「devUrl」；漏了的话打出来的包能装上，一打开却提示
>    连不上 `http://localhost:1420`（`tauri::is_dev()` 实际就是 `!cfg!(feature = "custom-protocol")`）。
> 2. NDK 的 `libc++_shared.so` 要和 `libconceit_lib.so` 一起放进 jniLibs，否则启动即崩。

**前置环境**（缺失时脚本会明确报错）：

| 变量 | 说明 |
|---|---|
| `JAVA_HOME` | JDK 17+ |
| `ANDROID_HOME` | Android SDK（需含 `platforms;android-37`、`build-tools`） |
| `ANDROID_NDK_HOME` | Android NDK（脚本会从里面取 `clang.cmd` 链接器和 `libc++_shared.so`） |

Rust 交叉编译目标只需装一次：

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi
```

**打包**：

```bash
node scripts/build-android.mjs              # 完整流程：前端 -> Rust -> 资源 -> APK
node scripts/build-android.mjs --skip-frontend   # 前端已构建过时
```

脚本在调用 Gradle 前会把 `src-tauri/android/*.kt`（应用自有的 Kotlin 源码——
原生桥 `ConceitPlugin.kt` 与拦截系统返回的 `MainActivity.kt`）同步进
`gen/android/app/src/main/java/`，
因此执行过 `tauri android init` 重建工程后也无需手动拷贝。

Gradle 的原始产物在 `src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk`，
打包脚本会额外复制一份到仓库根目录、按 `Conceit-<版本>-release.apk` 命名
（版本取自 `tauri.conf.json`，如 `Conceit-0.1.2-release.apk`），方便直接传手机安装；
arm64-v8a + armeabi-v7a 通用包，约 13 MB，minSdk 24 / Android 7.0+。

> Gradle 首次运行需要下载 Gradle 发行版与 AGP 等依赖；国内网络下 `services.gradle.org`
> 可能很慢，可先从镜像下载 `gradle-9.6.1-bin.zip` 解压到本地，脚本会优先使用本地安装。

**签名**：release 签名配置在 `gen/android/app/build.gradle.kts`，密钥信息从
`gen/android/keystore.properties` 读取（该文件含密码，已在 `.gitignore` 中忽略）。
首次配置：

```bash
keytool -genkeypair -v -keystore <密钥路径>/conceit-upload.jks -alias conceit \
  -keyalg RSA -keysize 2048 -validity 10000 \
  -storepass <密码> -keypass <密码> -dname "CN=Conceit, OU=Conceit, O=Conceit, C=CN"

# gen/android/keystore.properties（路径用正斜杠，Java 的 properties 会把 \ 当转义符）
storeFile=<密钥路径>/conceit-upload.jks
storePassword=<密码>
keyAlias=conceit
keyPassword=<密码>
```

> **请妥善保管密钥文件与密码**：以后每次更新安装到同一台手机，都必须用同一个密钥签名，
> 否则会被当成另一个应用而无法覆盖安装。密钥丢失且未上架应用商店时，只能卸载重装。
> 密钥与密码不要提交到 git。

**安装到手机**：开启「设置 → 安全 → 安装未知应用」，把 APK 传到手机点击即可；
或用数据线执行 `adb install -r Conceit-0.1.2-release.apk`。

> 打包脚本会把 Gradle 的原始产物（`app-universal-release.apk`，Gradle 默认的
> 模块名+变体命名）复制为 `Conceit-<版本>-release.apk` 放到仓库根目录，
> 与应用本身的名字无关——安装后图标和设置里显示的是 `strings.xml` 中的 `app_name`（Conceit），
> 应用标识为 `com.conceit.mangareader`。

## 书库目录结构

书库位于 `<应用数据目录>/library`，Windows 下通常为
`%APPDATA%\com.conceit.mangareader\library`：

```
library/
├── index.json          # 书架索引：条目、父子关系、每个容器内的排序、阅读进度
├── manga/
│   ├── m1/             # 每部漫画一个目录（导入时复制 / 解压生成）
│   │   ├── 001.jpg
│   │   ├── 002.jpg
│   │   └── ...
│   └── m2/
└── thumbs/
    ├── m1.jpg          # 封面缩略图（长边 480px，仅供书架封面使用）
    └── m2.jpg
```

- `index.json` 是应用唯一需要维护的状态文件；删除它可以重置书架（已复制的图片文件不受影响）
- 支持的图片格式：jpg / jpeg / png / webp / gif / bmp / avif
- 封面缩略图在导入时、以及每次更换封面页时自动生成；启动时会后台补生成缺失的缩略图
  （历史数据或被手动删除的文件可自愈，完成后前端自动刷新书架）。**阅读时加载的始终是原图**，
  画质不受影响；原图长边不足 480px 时不生成，书架直接显示原图

## 技术架构

```
src/                         前端（React 18 + Vite 5 + TypeScript 严格模式）
├── App.tsx                  应用状态：书架 / 阅读器切换、文件夹栈、拖拽导入、Toast
├── api.ts                   对 Rust 命令的封装 + 书架条目排序
├── types.ts                 与 Rust 数据模型一一对应的类型（含 Toast）
├── backHandler.ts           系统返回处理栈（模态框 / 阅读器按注册顺序消费）
├── usePageSorting.ts        漫画图片排序：鼠标拖放 + 触屏长按拖动
├── useSystemBarInsets.ts    系统栏高度 → CSS 变量（安卓原生桥）
├── styles/                  样式，按关注点拆分，由 index.css 统一引入
│   ├── base.css             变量、重置、应用外壳、按钮、通用动画、滚动条
│   ├── shelf.css            书架：底部功能栏、面包屑、卡片网格、条目卡片、右键菜单
│   ├── reader.css           阅读器：浮层底栏、缩放控件、进度滑块、滚动区
│   ├── overlay.css          忙碌遮罩、Toast、模态弹窗、表单、封面选页与图片管理
│   └── mobile.css           移动端 / 触屏适配（断点与 hover:none）
├── components/
│   ├── shelf/               书架功能域
│   │   ├── Shelf.tsx        编排：状态、弹窗、网格渲染
│   │   ├── ShelfHeader.tsx  底部功能栏：品牌 + 面包屑 + 操作按钮
│   │   ├── Card.tsx         漫画 / 文件夹卡片（封面 / 纯色封面、进度条、设置按钮）
│   │   ├── useShelfMutations.ts  导入 / 新建 / 删除 / 移动
│   │   └── useItemSorting.ts     鼠标拖放 + 触屏长按排序（共用落点计算）
│   ├── reader/              阅读器功能域
│   │   ├── Reader.tsx       图片加载、进度恢复与保存、缩放、键盘操作
│   │   └── ReaderBottombar.tsx  唯一底栏（退出 / 标题 / 缩放 / 进度滑块 / 计数）
│   ├── ItemSettings.tsx     条目设置（名称 / 封面开关 / 封面页选择 / 图片增删与排序）
│   └── ui/                  通用组件：Modal、InputDialog、ContextMenu、icons

src-tauri/                   后端（Rust + Tauri 2）
├── src/library.rs           书库模型：索引持久化、自然排序、目录复制、zip 解压、漫画数量统计、封面缩略图生成、图片增删辅助函数
├── src/commands.rs          Tauri 命令层（快照 / 增删改 / 排序 / 进度 / 导入 / 漫画图片管理）
├── src/mobile_import.rs     Android 原生桥（SAF 目录 / zip / 图片选择等，桌面端为存根）
├── android/ConceitPlugin.kt Android 原生桥的 Kotlin 实现（构建时同步进 gen/android）
├── android/MainActivity.kt  主 Activity：拦截系统返回转交前端（构建时同步进 gen/android）
└── tauri.conf.json          桌面配置（asset 协议加载本地图片）
```

### Android 导入桥

Android 上 `tauri-plugin-dialog` 有两个硬伤，直接用会报错：

1. **没有目录选择器**——JS 传 `directory: true` 时插件 Rust 端直接返回
   `FolderPickerNotImplemented`，前端表现为「打开选择对话框失败」；
2. **文件选择器返回的不是路径**——Android 端返回 `content://` URI 字符串，
   `std::fs` 无法读取，前端表现为「路径不存在: content://…」。

为此应用内置了一个 Kotlin 插件（`src-tauri/android/ConceitPlugin.kt`），用系统文档选择器
（SAF，`ACTION_OPEN_DOCUMENT_TREE` / `ACTION_GET_CONTENT`）完成
「选择 → 复制到应用缓存目录 → 返回真实路径」，Rust 侧之后按普通文件夹 / zip 处理。
复制时保留原始名称，导入后的漫画标题仍然是有意义的文件夹名 / 压缩包名；
整棵目录树的复制放在后台线程执行，避免主线程 ANR。前端通过 `app_platform` 命令
区分平台：桌面走插件对话框，Android 走原生桥。
设置页的「增加图片」多选走同一个插件的 `pickImages`（`ACTION_OPEN_DOCUMENT` +
`image/*` 多选），复制进 `picked-images` 缓存目录，由 `add_manga_pages`
复制进书库后随 `clear_import_cache` 一并清理。

> Kotlin 源码的规范副本在 `src-tauri/android/`，`scripts/build-android.mjs` 每次构建前
> 会把它同步进 `gen/android/app/src/main/java/com/conceit/mangareader/`
> （`tauri android init` 重建 gen/android 后无需手动拷贝）。仓库里同时保留了工作副本，
> 直接用官方 `tauri android build` 也能编到。

### 系统返回与系统栏适配

**系统返回**：`MainActivity` 在 `onCreate` 里向 `OnBackPressedDispatcher` 注册回调。
wry 因 `handleBackNavigation = false` 不注册自己的回调，不拦截时系统会直接结束
Activity——在阅读器里划边缘返回也会退出应用。拦截后把系统返回（硬件返回键与
边缘滑动手势走同一条链路）转发成前端的 `conceit:android-back` DOM 事件，
由 `src/App.tsx` 按当前状态决定去向：

1. 阅读器打开 → 回到书架（阅读器卸载时自动保存进度）
2. 书架内 → 文件夹逐级返回上一层
3. 根目录 → 调 `exit_app` 退出（明确选择「直接退出」而非「再按一次退出」）

弹窗与操作菜单通过 `src/backHandler.ts` 的返回处理栈优先消费，不会被误退出。

**系统栏避让**：安卓 WebView 的 `env(safe-area-inset-*)` 在部分设备上返回 0，
顶栏会被状态栏盖住。`system_bar_insets` 命令经 `WindowInsetsCompat` 取真实高度，
`useSystemBarInsets` 在启动与每次 `resize`（旋屏、折叠屏形态切换——Activity 声明了
`configChanges`，页面不重载）后把它同步为 `--status-bar-h` / `--nav-bar-h` 两个
CSS 变量；`:root` 里的默认值仍取 `env()`，桌面 / iOS 行为不变。
书架功能栏固定底部后，顶部不再为状态栏留白，网格直接顶到屏幕边缘。

### 漫画图片管理

条目设置（`src/components/ItemSettings.tsx`）里，漫画的缩略图网格同时承担三件事：
点一下设为封面、点右上角 ✕ 标记删除、拖动调整顺序。对应三条 Rust 命令：

| 命令 | 行为 |
|---|---|
| `add_manga_pages` | 把所选图片复制进 `library/manga/<id>/`（重名自动加序号），**追加到 `pages` 末尾**——刻意不插队，追加后拖动归位即可，不会打乱已自定义的顺序 |
| `delete_manga_pages` | 索引与磁盘文件一并删除；封面页被删时顺延到当前第一页并重建缩略图；**至少保留一页**（要清空请直接删除整部漫画） |
| `reorder_manga_pages` | 提交的必须与当前是同一组图片（`same_page_set` 校验），防止把增删伪装成重排传进来 |

删除是「先标记、后统一确认」的两步操作：✕ 标记后缩略图变红并显示「待删除」，
section 头部出现「删除所选 N 页」，再点一次确认才真正删文件，防误触。
排序用 `src/usePageSorting.ts`（鼠标 HTML5 拖放 + 触屏长按拖动，与书架排序同一套手势），
先本地生效，保存失败自动回滚。

### 实现要点

- **自维护书库**：导入即把图片复制进 `library/manga/<id>/`，书库与源目录完全独立
- **asset 协议**：前端通过 `convertFileSrc` 直接加载书库内的本地图片，无需额外读取权限
- **自然排序**：`page2.jpg` 排在 `page10.jpg` 之前，符合漫画页码习惯
- **封面策略**：默认纯色封面（按条目 id 派生稳定色值），可在设置中开启封面图并任选一页作为封面
- **进度恢复**：退出或滚动时节流写入 `index.json`，下次打开按比例恢复滚动位置
- **文件夹即容器**：每个容器（根目录或文件夹）独立维护排序，移动条目只改变父子关系
- **兼容旧索引**：`show_cover`、`thumb` 等新字段均带 serde 默认值，旧 `index.json` 可直接读取
- **Android 导入桥**：插件对话框在 Android 上既不能选目录、也只返回 `content://` URI，
  由内置 Kotlin 插件经 SAF 拿到缓存里的真实路径后再走同一套导入逻辑（详见上文）
- **系统返回**：`MainActivity` 拦截系统返回并转交前端，阅读器→书架、文件夹逐级返回、
  根目录才退出；弹窗经返回处理栈优先消费（详见上文）
- **系统栏避让**：状态栏 / 导航栏高度经原生桥同步为 CSS 变量，兜底 WebView
  `env(safe-area-inset-*)` 在部分安卓设备上取不到值的问题（详见上文）
- **书架功能栏贴底**：导入 / 新建 / 面包屑移到屏幕底部，平板与手机上拇指更好够到，
  顶部不再为状态栏留白（详见上文）
- **漫画图片管理**：设置页可增加 / 删除图片与拖动排序，追加不插队、删除两步确认、
  封面页被删自动顺延（详见上文）
- **Android 打包**：`tauri android build` 在 Windows 上受符号链接权限所限，改用
  `scripts/build-android.mjs`（文件复制 + 直接调 Gradle），签名走 `keystore.properties`

## 已知限制

- 超大漫画（数百页）会一次性加载全部图片，内存占用较高；分页/懒加载是后续可做的优化
- 条目设置里的「选页封面」网格同样加载原图（带 `loading="lazy"`），页数很多时偏重；
  后续可改为生成每页的小图
- 缩略图只覆盖 jpg / png / webp / gif / bmp；avif 等格式暂不生成（书架回退显示原图封面）
- Android 导入会先把所选文件复制进应用缓存目录、再复制进书库（SAF 的限制，无法直接读源路径），
  大漫画的导入耗时与临时占用略高；导入完成后会自动清理缓存
- APK 目前只打包 arm64-v8a 与 armeabi-v7a 两个 ABI（覆盖真实手机设备）；
  模拟器用的 x86 / x86_64 未包含，需要时在 `scripts/build-android.mjs` 的 `TARGETS` 里补

## 命名

| | |
|---|---|
| 包名 / 产品名 | **Conceit** |
| 中文名 | **幻想阅读器** |
| 应用标识 | `com.conceit.mangareader` |
