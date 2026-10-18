/**
 * 封面设置端到端验证（CDP + 触摸事件）：
 *  - 打开条目设置 → 勾选「展示封面图片」→ 保存
 *  - 书架卡片应渲染封面 <img>（而非纯色块）
 *  - 重新打开设置，复选框应保持勾选（读取路径与保存路径一致）
 *  - 取消勾选 → 保存 → 卡片恢复纯色封面
 * 该脚本是「Tauri 响应体键名」回归的防线：曾因 TS 侧声明 showCover
 * 而 Rust 响应为 show_cover，导致封面开关能存不生效。
 */
const BASE = "http://127.0.0.1:9222";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const results = [];
function check(name, ok, detail = "") {
  results.push({ name, ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? `  [${detail}]` : ""}`);
}

async function main() {
  const list = await fetch(`${BASE}/json/list`).then((r) => r.json());
  const page = list.find((t) => t.type === "page" && t.url.includes("1420"));
  if (!page) {
    console.log("未找到页面 target");
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
    const m = JSON.parse(ev.data);
    if (m.id && pending.has(m.id)) {
      const { resolve, reject } = pending.get(m.id);
      pending.delete(m.id);
      m.error ? reject(new Error(JSON.stringify(m.error))) : resolve(m.result);
    }
  };
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const mid = ++id;
      pending.set(mid, { resolve, reject });
      ws.send(JSON.stringify({ id: mid, method, params }));
      setTimeout(() => reject(new Error(`timeout: ${method}`)), 20000);
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
  const touch = async (type, x, y) =>
    send("Input.dispatchTouchEvent", {
      type,
      touchPoints: type === "touchEnd" ? [] : [{ x, y, id: 0 }],
    });
  const tap = async (x, y) => {
    await touch("touchStart", x, y);
    await sleep(40);
    await touch("touchEnd", x, y);
  };
  /** 点某个选择器命中元素的中心 */
  const tapEl = async (selector, nth = 0) => {
    const p = await evalJs(`(() => {
      const els = document.querySelectorAll(${JSON.stringify(selector)});
      const el = els[${nth}];
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
    })()`);
    if (!p) return false;
    await tap(p.x, p.y);
    return true;
  };

  await send("Runtime.enable");
  await send("Emulation.setDeviceMetricsOverride", {
    width: 390,
    height: 844,
    deviceScaleFactor: 3,
    mobile: true,
  });
  await send("Emulation.setTouchEmulationEnabled", { enabled: true });
  await sleep(500);

  // 回到根目录
  const crumb = await evalJs(`(() => {
    const crumbs = document.querySelectorAll('.crumb');
    if (crumbs.length <= 1) return null;
    const r = crumbs[0].getBoundingClientRect();
    return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
  })()`);
  if (crumb) {
    await tap(crumb.x, crumb.y);
    await sleep(500);
  }

  const mangaId = await evalJs(
    `document.querySelector('.card-manga')?.dataset.cardId ?? null`
  );
  if (!mangaId) {
    console.log("根目录没有漫画卡片，无法验证");
    ws.close();
    process.exit(1);
  }

  // 卡片当前是否展示封面图
  const cardHasCoverImg = () =>
    evalJs(`!!document.querySelector('.card-manga .card-cover img')`);

  /* ---- 1. 打开设置，初始状态 ---- */
  await tapEl(`[data-card-id="${mangaId}"] .card-settings`);
  await sleep(600);
  const modalOpen = await evalJs(`!!document.querySelector('.modal')`);
  check("打开条目设置弹窗", modalOpen);

  const initialChecked = await evalJs(
    `document.querySelector('.switch-row input')?.checked ?? null`
  );
  check("设置弹窗读取到封面开关", initialChecked === true || initialChecked === false, `checked=${initialChecked}`);

  /* ---- 2. 勾选并保存 ---- */
  await tapEl(".switch-row input");
  await sleep(300);
  const toggled = await evalJs(
    `document.querySelector('.switch-row input')?.checked`
  );
  await tapEl(".modal-footer .btn-primary");
  let closed = false;
  for (let i = 0; i < 20; i += 1) {
    await sleep(300);
    closed = await evalJs(`!document.querySelector('.modal')`);
    if (closed) break;
  }
  check("保存后弹窗关闭", closed);
  await sleep(600); // 等待书架刷新

  const hasImg = await cardHasCoverImg();
  check(
    `勾选展示封面后卡片渲染封面图`,
    hasImg === (toggled === true),
    `勾选=${toggled} 卡片有封面图=${hasImg}`
  );
  const coverSrc = await evalJs(
    `document.querySelector('.card-manga .card-cover img')?.src ?? null`
  );
  // asset 协议会把路径编码，先解码再判断
  const decodedSrc = coverSrc ? decodeURIComponent(coverSrc) : null;
  check(
    "封面加载的是缩略图而非原图",
    decodedSrc == null || decodedSrc.includes("/thumbs/"),
    decodedSrc ?? "无封面图"
  );

  /* ---- 3. 重新打开，状态应持久 ---- */
  await tapEl(`[data-card-id="${mangaId}"] .card-settings`);
  await sleep(600);
  const reopened = await evalJs(
    `document.querySelector('.switch-row input')?.checked ?? null`
  );
  check(
    "重新打开设置时复选框反映已保存状态",
    reopened === toggled,
    `期望=${toggled} 实际=${reopened}`
  );
  await tapEl(".modal-footer .btn"); // 取消，保持当前状态
  await sleep(400);

  /* ---- 4. 还原：取消勾选 ---- */
  await tapEl(`[data-card-id="${mangaId}"] .card-settings`);
  await sleep(600);
  const before = await evalJs(
    `document.querySelector('.switch-row input')?.checked`
  );
  if (before === true) {
    await tapEl(".switch-row input");
    await sleep(300);
  }
  await tapEl(".modal-footer .btn-primary");
  for (let i = 0; i < 20; i += 1) {
    await sleep(300);
    if (await evalJs(`!document.querySelector('.modal')`)) break;
  }
  await sleep(600);
  const restored = await cardHasCoverImg();
  check("还原为纯色封面", restored === false, `卡片有封面图=${restored}`);

  try {
    await send("Emulation.clearDeviceMetricsOverride");
    await send("Emulation.setTouchEmulationEnabled", { enabled: false });
  } catch {}
  const failed = results.filter((r) => !r.ok);
  console.log(`\n结果：${results.length - failed.length}/${results.length} 通过`);
  ws.close();
  process.exit(failed.length ? 1 : 0);
}

main().catch((e) => {
  console.error("验证脚本异常：", e);
  process.exit(1);
});
