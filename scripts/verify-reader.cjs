/**
 * 阅读器行为验证（通过 WebView2 远程调试 CDP 驱动真实 UI）：
 *  1. 打开漫画卡片
 *  2. 顶部栏为浮层（position: absolute），滚动区始终铺满全屏
 *  3. 点击画面显隐工具栏；隐藏时首图紧贴顶部（padding-top: 0），无黑边
 *  4. Ctrl + 滚轮缩放（百分比变化），普通滚轮不缩放
 */
const BASE = "http://127.0.0.1:9222";

const results = [];
function check(name, ok, detail = "") {
  results.push({ name, ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? `  [${detail}]` : ""}`);
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  let list;
  for (let i = 0; i < 20; i += 1) {
    try {
      list = await fetch(`${BASE}/json/list`).then((r) => r.json());
      if (list.some((t) => t.type === "page" && t.url.includes("localhost:1420"))) break;
    } catch {}
    await sleep(1000);
  }
  const page = (list || []).find(
    (t) => t.type === "page" && t.url.includes("localhost:1420")
  );
  if (!page) {
    console.log("未找到页面 target：", JSON.stringify((list || []).map((t) => t.url)));
    process.exit(1);
  }

  const ws = new WebSocket(page.webSocketDebuggerUrl);
  let id = 0;
  const pending = new Map();
  await new Promise((res, rej) => {
    ws.onopen = res;
    ws.onerror = () => rej(new Error("ws error"));
  });
  ws.onmessage = (ev) => {
    const msg = JSON.parse(ev.data);
    if (msg.id && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id);
      pending.delete(msg.id);
      if (msg.error) reject(new Error(JSON.stringify(msg.error)));
      else resolve(msg.result);
    }
  };
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const mid = ++id;
      pending.set(mid, { resolve, reject });
      ws.send(JSON.stringify({ id: mid, method, params }));
      setTimeout(() => {
        if (pending.has(mid)) {
          pending.delete(mid);
          reject(new Error(`timeout: ${method}`));
        }
      }, 15000);
    });
  const evalJs = async (expression) => {
    const r = await send("Runtime.evaluate", {
      expression,
      returnByValue: true,
      awaitPromise: true,
    });
    if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails));
    return r.result?.value;
  };
  const click = async (x, y) => {
    for (const type of ["mousePressed", "mouseReleased"]) {
      await send("Input.dispatchMouseEvent", {
        type,
        x,
        y,
        button: "left",
        clickCount: 1,
        buttons: type === "mousePressed" ? 1 : 0,
      });
    }
  };
  const wheel = async (x, y, deltaY, ctrl = false) => {
    await send("Input.dispatchMouseEvent", {
      type: "mouseWheel",
      x,
      y,
      deltaX: 0,
      deltaY,
      modifiers: ctrl ? 2 : 0, // Ctrl = 2
    });
  };

  await send("Runtime.enable");
  await send("Page.enable");

  /* ---------- 书架状态 ---------- */
  const cards = await evalJs(
    `document.querySelectorAll('.card').length`
  );
  check("书架渲染出卡片", cards >= 2, `cards=${cards}`);

  /* ---------- 打开漫画 ---------- */
  const rect = await evalJs(`(() => {
    const el = document.querySelector('.card-manga');
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
  })()`);
  if (!rect) {
    console.log("未找到漫画卡片，终止");
    process.exit(1);
  }
  await click(rect.x, rect.y);

  let opened = false;
  for (let i = 0; i < 30; i += 1) {
    await sleep(300);
    opened = await evalJs(`!!document.querySelector('.reader')`);
    if (opened) break;
  }
  check("点击卡片后阅读器打开", opened);

  /* ---------- 浮层与铺满 ---------- */
  await sleep(1200); // 等图片就位
  const layout = await evalJs(`(() => {
    const bar = document.querySelector('.reader-topbar');
    const scroll = document.querySelector('.reader-scroll');
    const track = document.querySelector('.reader-track');
    const cs = getComputedStyle(bar);
    const ss = getComputedStyle(scroll);
    return {
      barPosition: cs.position,
      scrollHeight: scroll.getBoundingClientRect().height,
      winHeight: window.innerHeight,
      scrollTop: scroll.getBoundingClientRect().top,
      trackPaddingTop: getComputedStyle(track).paddingTop,
      pages: document.querySelectorAll('.reader-page').length,
      firstImgLoaded: (() => {
        const img = document.querySelector('.reader-page');
        return img ? img.naturalWidth > 0 : false;
      })(),
    };
  })()`);
  check(
    "顶部栏为绝对定位浮层",
    layout.barPosition === "absolute",
    `position=${layout.barPosition}`
  );
  check(
    "滚动区铺满整个屏幕高度",
    Math.abs(layout.scrollHeight - layout.winHeight) < 1 &&
      Math.abs(layout.scrollTop) < 1,
    `scroll=${layout.scrollHeight} win=${layout.winHeight} top=${layout.scrollTop}`
  );
  check("漫画图片已加载", layout.pages >= 1 && layout.firstImgLoaded, `pages=${layout.pages}`);

  /* ---------- 点击显隐 ---------- */
  const cx = await evalJs(`window.innerWidth / 2`);
  const cy = await evalJs(`window.innerHeight * 0.65`);
  const topBefore = await evalJs(
    `document.querySelector('.reader-page').getBoundingClientRect().top`
  );
  await click(cx, cy); // 隐藏
  await sleep(400);
  const hidden = await evalJs(`(() => {
    const bar = document.querySelector('.reader-topbar');
    const track = document.querySelector('.reader-track');
    return {
      hasClass: bar.classList.contains('chrome-hidden'),
      opacity: getComputedStyle(bar).opacity,
      trackPaddingTop: getComputedStyle(track).paddingTop,
      scrollHeight: document.querySelector('.reader-scroll').getBoundingClientRect().height,
      winHeight: window.innerHeight,
      pageTop: document.querySelector('.reader-page').getBoundingClientRect().top,
    };
  })()`);
  check("再次点击画面隐藏工具栏", hidden.hasClass, `opacity=${hidden.opacity}`);
  check(
    "隐藏时首图紧贴顶部（无黑边）",
    hidden.trackPaddingTop === "0px",
    `padding-top=${hidden.trackPaddingTop}`
  );
  check(
    "隐藏时滚动区仍然铺满全屏",
    Math.abs(hidden.scrollHeight - hidden.winHeight) < 1,
    `scroll=${hidden.scrollHeight}`
  );
  check(
    "隐藏工具栏不移动漫画内容",
    Math.abs(hidden.pageTop - topBefore) < 0.5,
    `pageTop ${topBefore} -> ${hidden.pageTop}`
  );

  await click(cx, cy); // 显示
  await sleep(400);
  const shown = await evalJs(`(() => ({
    hasClass: document.querySelector('.reader-topbar').classList.contains('chrome-hidden'),
    trackPaddingTop: getComputedStyle(document.querySelector('.reader-track')).paddingTop,
    pageTop: document.querySelector('.reader-page').getBoundingClientRect().top,
  }))()`);
  check("第三次点击恢复工具栏", !shown.hasClass);
  check(
    "顶部内边距恒定（不随显隐变化）",
    shown.trackPaddingTop === "0px",
    `padding-top=${shown.trackPaddingTop}`
  );
  check(
    "显示工具栏不移动漫画内容",
    Math.abs(shown.pageTop - topBefore) < 0.5,
    `pageTop ${topBefore} -> ${shown.pageTop}`
  );

  /* ---------- Ctrl + 滚轮缩放 ---------- */
  const zoomText = () =>
    evalJs(`document.querySelector('.reader-zoom-value')?.textContent ?? ''`);
  const before = await zoomText();
  await wheel(cx, cy, -120, true); // Ctrl + 上滚 → 放大
  await sleep(300);
  const afterIn = await zoomText();
  check(
    "Ctrl+滚轮上滑放大",
    parseInt(afterIn, 10) > parseInt(before, 10),
    `${before} -> ${afterIn}`
  );

  const beforePlain = afterIn;
  await wheel(cx, cy, 120, false); // 普通滚轮 → 只滚动不缩放
  await sleep(300);
  const afterPlain = await zoomText();
  check(
    "普通滚轮不改变缩放",
    afterPlain === beforePlain,
    `${beforePlain} -> ${afterPlain}`
  );

  await wheel(cx, cy, 240, true); // Ctrl + 下滚 → 缩小
  await sleep(300);
  const afterOut = await zoomText();
  check(
    "Ctrl+滚轮下滑缩小",
    parseInt(afterOut, 10) < parseInt(afterPlain, 10),
    `${afterPlain} -> ${afterOut}`
  );

  /* ---------- 退出阅读器 ---------- */
  await evalJs(
    `document.querySelector('.reader-topbar .btn')?.click()`
  );
  await sleep(400);
  const closed = await evalJs(`!document.querySelector('.reader')`);
  check("退出按钮可退出阅读器", closed);

  const failed = results.filter((r) => !r.ok);
  console.log(`\n结果：${results.length - failed.length}/${results.length} 通过`);
  ws.close();
  process.exit(failed.length ? 1 : 0);
}

main().catch((e) => {
  console.error("验证脚本异常：", e);
  process.exit(1);
});
