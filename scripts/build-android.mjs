#!/usr/bin/env node
// @ts-check
/// <reference types="node" />
/**
 * Conceit Android 打包脚本。
 *
 * 为什么不直接用 `tauri android build`：
 * 该命令在把 Rust 编出的 .so 软链到 gen/android/app/src/main/jniLibs/ 时会调用
 * Windows 的 CreateSymbolicLinkW，而这个 API 需要「开发者模式」或管理员权限。
 * 在没有这些权限的机器上整条命令会中断（Rust 已编译成功，只差最后链接那步）。
 *
 * 本脚本用普通文件复制完成同样的搬运，再直接调用 Gradle 打包，
 * 产物与官方流程一致（同样的 release profile、同样的签名配置）。
 *
 * 前置环境（缺失时脚本会给出明确提示）：
 *   JAVA_HOME          JDK 17+
 *   ANDROID_HOME       Android SDK
 *   ANDROID_NDK_HOME   Android NDK
 *
 * 用法：node scripts/build-android.mjs [--skip-frontend]
 */

import { execSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  copyFileSync,
  readFileSync,
  writeFileSync,
  rmSync,
  readdirSync,
  statSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SRC_TAURI = path.join(ROOT, "src-tauri");
const GEN_ANDROID = path.join(SRC_TAURI, "gen", "android");
const APP_DIR = path.join(GEN_ANDROID, "app");
const JNI_LIBS = path.join(APP_DIR, "src", "main", "jniLibs");
const ASSETS = path.join(APP_DIR, "src", "main", "assets");
const APP_JAVA = path.join(APP_DIR, "src", "main", "java");
/** 应用自有的 Kotlin 源码（tauri android init 重建 gen/android 时不会生成它们） */
const ANDROID_KT_SRC = path.join(SRC_TAURI, "android");
const DIST = path.join(ROOT, "dist");

/** minSdk，决定 NDK 链接器包装脚本的 API 级别 */
const MIN_API = 24;

/** 目标三元组 -> Android ABI 的映射 */
const TARGETS = [
  {
    rust: "aarch64-linux-android",
    abi: "arm64-v8a",
    sysroot: "aarch64-linux-android",
    clang: `aarch64-linux-android${MIN_API}-clang.cmd`,
  },
  {
    rust: "armv7-linux-androideabi",
    abi: "armeabi-v7a",
    sysroot: "arm-linux-androideabi",
    clang: `armv7a-linux-androideabi${MIN_API}-clang.cmd`,
  },
];

const skipFrontend = process.argv.includes("--skip-frontend");

function log(msg) {
  console.log(`\n[android] ${msg}`);
}

function die(msg) {
  console.error(`\n[android] 错误：${msg}`);
  process.exit(1);
}

function checkEnv() {
  const required = {
    JAVA_HOME: process.env.JAVA_HOME,
    ANDROID_HOME: process.env.ANDROID_HOME,
    ANDROID_NDK_HOME: process.env.ANDROID_NDK_HOME,
  };
  const missing = Object.entries(required)
    .filter(([, v]) => !v || !existsSync(v))
    .map(([k]) => k);
  if (missing.length) {
    die(
      `缺少环境变量 ${missing.join("、")}。\n` +
        `  JAVA_HOME        指向 JDK 17+\n` +
        `  ANDROID_HOME     指向 Android SDK\n` +
        `  ANDROID_NDK_HOME 指向 Android NDK\n` +
        `详见 README「打包 Android APK」一节。`
    );
  }
  if (!existsSync(GEN_ANDROID)) {
    die(
      `未找到 ${GEN_ANDROID}，请先执行：\n` +
        `  pnpm tauri android init --ci --skip-targets-install`
    );
  }
  return required;
}

/** 编译 Rust 动态库（不经过 tauri CLI，避免符号链接） */
function buildRust() {
  const ndk = process.env.ANDROID_NDK_HOME;
  const bin = path.join(ndk, "toolchains", "llvm", "prebuilt", "windows-x86_64", "bin");
  for (const t of TARGETS) {
    log(`编译 Rust 库：${t.rust}`);
    const linker = path.join(bin, t.clang);
    if (!existsSync(linker)) {
      die(`找不到 NDK 链接器 ${linker}`);
    }
    // custom-protocol 决定 tauri 走「打包进 assets 的自定义协议」还是「devUrl」：
    // 官方 `tauri android build` 会传这个特性，漏了的话打出来的包会去连开发服务器。
    execSync(
      `cargo build --release --lib --target ${t.rust} --features tauri/custom-protocol`,
      {
        cwd: SRC_TAURI,
        stdio: "inherit",
        env: {
          ...process.env,
          NDK_HOME: ndk,
          [`CARGO_TARGET_${t.rust.toUpperCase().replace(/-/g, "_")}_LINKER`]: linker,
        },
      }
    );
  }
}

/** 把 .so 与 NDK 的 libc++_shared.so 放进 jniLibs */
function copyNativeLibs() {
  const ndk = process.env.ANDROID_NDK_HOME;
  const sysroot = path.join(ndk, "toolchains", "llvm", "prebuilt", "windows-x86_64", "sysroot", "usr", "lib");
  for (const t of TARGETS) {
    const dest = path.join(JNI_LIBS, t.abi);
    mkdirSync(dest, { recursive: true });
    const so = path.join(SRC_TAURI, "target", t.rust, "release", "libconceit_lib.so");
    if (!existsSync(so)) die(`找不到 ${so}，Rust 编译似乎没成功`);
    copyFileSync(so, path.join(dest, "libconceit_lib.so"));

    // Rust 的 Android 二进制动态依赖 libc++_shared.so，缺了会在启动时崩溃
    const cxx = path.join(sysroot, t.sysroot, "libc++_shared.so");
    if (!existsSync(cxx)) die(`找不到 ${cxx}`);
    copyFileSync(cxx, path.join(dest, "libc++_shared.so"));
    log(`已放入 jniLibs/${t.abi}/（libconceit_lib.so + libc++_shared.so）`);
  }
}

/** 把前端产物与 tauri.conf.json 放进 Android assets（tauri CLI 原本做的两步） */
function copyAssets() {
  if (!existsSync(DIST)) die(`找不到 ${DIST}，请先执行 pnpm build`);
  rmSync(ASSETS, { recursive: true, force: true });
  mkdirSync(ASSETS, { recursive: true });
  for (const entry of readdirSync(DIST)) {
    copyDir(path.join(DIST, entry), path.join(ASSETS, entry));
  }
  copyFileSync(path.join(SRC_TAURI, "tauri.conf.json"), path.join(ASSETS, "tauri.conf.json"));
  log(`已放入 assets/（dist 全部内容 + tauri.conf.json）`);
}

function copyDir(src, dest) {
  const st = statSync(src);
  if (st.isDirectory()) {
    mkdirSync(dest, { recursive: true });
    for (const entry of readdirSync(src)) copyDir(path.join(src, entry), path.join(dest, entry));
  } else {
    mkdirSync(path.dirname(dest), { recursive: true });
    copyFileSync(src, dest);
  }
}

/** 把 src-tauri/android/ 下的 Kotlin 源码同步进 gen/android。
 *  ConceitPlugin.kt 是 tauri 不会生成的应用自有文件；MainActivity.kt 虽由
 *  `tauri android init` 生成，但我们改写了它（拦截系统返回转交前端），
 *  必须以 src-tauri/android/ 的规范副本为准。此步骤保证两者一致，
 *  init 重建工程后也可自愈。 */
function syncAndroidSources() {
  if (!existsSync(ANDROID_KT_SRC)) return;
  const destDir = path.join(APP_JAVA, "com", "conceit", "mangareader");
  for (const entry of readdirSync(ANDROID_KT_SRC)) {
    if (!entry.endsWith(".kt")) continue;
    mkdirSync(destDir, { recursive: true });
    copyFileSync(path.join(ANDROID_KT_SRC, entry), path.join(destDir, entry));
    log(`已同步应用 Kotlin 源码：${entry}`);
  }
}

/** 找一个可用的 Gradle：本地安装优先（wrapper 每次都要下载发行包，国内很慢） */
function findGradle() {
  const local = "C:\\Users\\LRY\\AndroidTools\\gradle-9.6.1\\bin\\gradle.bat";
  if (existsSync(local)) return local;
  try {
    execSync("gradle --version", { stdio: "ignore" });
    return "gradle";
  } catch {
    /* 不在 PATH 上 */
  }
  const wrapper = path.join(GEN_ANDROID, "gradlew.bat");
  if (existsSync(wrapper)) return wrapper;
  die("找不到 Gradle。请安装 Gradle 或使用仓库内的 gradlew.bat");
}

function runGradle() {
  log("调用 Gradle 打包（排除会回调 tauri CLI 的 rust 任务）");
  const gradle = findGradle();
  const args = [
    "assembleUniversalRelease",
    "-PabiList=arm64-v8a,armeabi-v7a",
    "-xrustBuildUniversalRelease",
    "-xrustBuildArm64Release",
    "-xrustBuildArmRelease",
    "-xrustBuildX86Release",
    "-xrustBuildX86_64Release",
  ];
  execSync(`"${gradle}" ${args.map((a) => `"${a}"`).join(" ")}`, {
    cwd: GEN_ANDROID,
    stdio: "inherit",
  });
}

/** 从 tauri.conf.json 读应用版本（版本号的唯一来源） */
function appVersion() {
  const conf = JSON.parse(
    readFileSync(path.join(SRC_TAURI, "tauri.conf.json"), "utf8")
  );
  if (!conf.version) die("tauri.conf.json 里没有 version 字段");
  return conf.version;
}

/** 同步 APK 的 versionName / versionCode 到 tauri.conf.json 的版本。
 *
 *  正常情况下 `tauri android init/build` 会重写 gen/android/app/tauri.properties，
 *  但本脚本绕过 CLI 直接调 Gradle，那个文件就永远停在初次 init 时的值
 *  （这个仓库从 0.1.2 起就没更新过，0.1.3 / 0.1.4 的包内部都还是 0.1.2/1002）。
 *  版本号不同步的后果：系统设置里显示旧版本，且无法判断新旧。
 *
 *  versionCode 必须单调递增，否则 `adb install -r` 会被判为降级而拒绝安装。
 *  这里按语义化版本算一个理论值，再和文件里的现值取较大者——历史包的 1002
 *  比公式值大，所以先延续 prev + 1，之后每次构建都会继续往上走。 */
function syncAndroidVersion() {
  const [major, minor, patch] = appVersion()
    .split(".")
    .map((n) => Number.parseInt(n, 10) || 0);
  const formula = major * 10000 + minor * 100 + patch;
  const propFile = path.join(APP_DIR, "tauri.properties");
  let prev = 0;
  if (existsSync(propFile)) {
    const m = readFileSync(propFile, "utf8").match(/versionCode=(\d+)/);
    if (m) prev = Number.parseInt(m[1], 10) || 0;
  }
  const versionCode = Math.max(formula, prev + 1);
  writeFileSync(
    propFile,
    "// THIS IS AN AUTOGENERATED FILE. DO NOT EDIT THIS FILE DIRECTLY.\n" +
      `tauri.android.versionName=${appVersion()}\n` +
      `tauri.android.versionCode=${versionCode}\n`
  );
  log(`APK 版本：${appVersion()}（versionCode=${versionCode}）`);
}

/** 把 Gradle 产出的 app-universal-release.apk 复制为
 *  Conceit-<版本>-release.apk（放仓库根目录，方便直接传手机安装） */
function publishApk(gradleApk, version) {
  const friendly = path.join(ROOT, `Conceit-${version}-release.apk`);
  copyFileSync(gradleApk, friendly);
  log(`完成：${friendly}`);
  log(`（Gradle 原始产物：${gradleApk}）`);
}

function main() {
  checkEnv();
  if (!skipFrontend) {
    log("构建前端");
    execSync("pnpm build", { cwd: ROOT, stdio: "inherit" });
  }
  buildRust();
  copyNativeLibs();
  copyAssets();
  syncAndroidSources();
  syncAndroidVersion();
  runGradle();
  const apk = path.join(
    APP_DIR,
    "build",
    "outputs",
    "apk",
    "universal",
    "release",
    "app-universal-release.apk"
  );
  if (!existsSync(apk)) die(`构建似乎成功了但找不到 ${apk}`);
  publishApk(apk, appVersion());
}

main();
