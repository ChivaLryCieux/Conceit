/**
 * 移动端适配验证（CDP + 设备指标模拟 + 触摸事件合成）：
 *  - 多视口（小屏手机 / 手机 / 平板）无横向溢出、布局正确
 *  - 触屏下设置按钮常显、无 hover 粘滞
 *  - 阅读器顶栏在窄屏两行排布、图片铺满、点击显隐不移动内容
 *  - 模态框在手机上变为底部弹出面板
 *  - 长按打开操作菜单（且不误触发打开漫画）
 *  - 长按拖动排序（往返一次，验证顺序变化并还原）
 */
const BASE = "http://127.0.0.1:9222";

const results = [];
function check(name, ok, detail = "") {
  results.push({ name, ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? `  [${detail}]` : ""}`);
}
function skip(name, reason) {
  console.log(`SKIP  ${name}  [${reason}]`);
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  let list;
  for (let i = 0; i < 20; i += 1) {
    try {
      list = await fetch(`${BASE}/json/list`).then((r) => r.json());
      if (list.some((t) => t.type === "page" && t.url.includes("localhost:1420")))
        break;
    } catch {}
    await sleep(1000);
  }
  const page = (list || []).find(
    (t) => t.type === "page" && t.url.includes("localhost:1420")
  );
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
      }, 20000);
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

  await send("Runtime.enable");
  await send("Page.enable");

  /* ---------- 触摸事件工具 ---------- */
  const touch = async (type, x, y) => {
    await send("Input.dispatchTouchEvent", {
      type,
      touchPoints: type === "touchEnd" ? [] : [{ x, y, id: 0 }],
    });
  };
  /** 长按：按住 → 等长按触发 → 可选拖动到 (mx,my) → 松手 */
  const longPress = async (x, y, mx, my) => {
    await touch("touchStart", x, y);
    await sleep(620);
    if (mx != null && my != null) {
      const steps = 8;
      for (let i = 1; i <= steps; i += 1) {
        await touch(
          "touchMove",
          x + ((mx - x) * i) / steps,
          y + ((my - y) * i) / steps
        );
        await sleep(16);
      }
    }
    await touch("touchEnd", mx ?? x, my ?? y);
  };
  const tap = async (x, y) => {
    await touch("touchStart", x, y);
    await sleep(30);
    await touch("touchEnd", x, y);
  };

  const cardRects = () =>
    evalJs(`Array.from(document.querySelectorAll('[data-card-id]')).map(el => {
      const r = el.getBoundingClientRect();
      return { id: el.dataset.cardId, manga: el.classList.contains('card-manga'),
               x: r.left, y: r.top, w: r.width, h: r.height,
               cx: r.left + r.width / 2, cy: r.top + r.height / 2 };
    })`);
  const cardOrder = () =>
    evalJs(`Array.from(document.querySelectorAll('[data-card-id]')).map(el => el.dataset.cardId).join(',')`);
  /** 回到根目录（防止上一轮测试残留在文件夹里） */
  const goRoot = async () => {
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
  };

  const VIEWPORTS = [
    { name: "小屏手机 360x640", width: 360, height: 640, dsf: 3, full: true },
    { name: "手机 390x844", width: 390, height: 844, dsf: 3, full: true },
    { name: "平板 834x1112", width: 834, height: 1112, dsf: 2, full: false },
  ];

  for (const vp of VIEWPORTS) {
    console.log(`\n===== ${vp.name} =====`);
    await send("Emulation.setDeviceMetricsOverride", {
      width: vp.width,
      height: vp.height,
      deviceScaleFactor: vp.dsf,
      mobile: true,
    });
    await send("Emulation.setTouchEmulationEnabled", { enabled: true });
    await sleep(500);

    /* ---------- 书架布局 ---------- */
    const layout = await evalJs(`(() => {
      const de = document.documentElement;
      const gear = document.querySelector('.card-settings');
      const grid = document.querySelector('.shelf-grid');
      const actions = document.querySelector('.shelf-actions');
      return {
        overflowX: de.scrollWidth - window.innerWidth,
        gearOpacity: gear ? getComputedStyle(gear).opacity : null,
        gearSize: gear ? gear.getBoundingClientRect().width : 0,
        gridCols: grid ? getComputedStyle(grid).gridTemplateColumns.split(' ').length : 0,
        gridWidth: grid ? grid.getBoundingClientRect().width : 0,
        actionsWidth: actions ? actions.getBoundingClientRect().width : 0,
        innerWidth: window.innerWidth,
        hoverNone: matchMedia('(hover: none)').matches,
        coverLift: (() => {
          const c = document.querySelector('.card-cover');
          return c ? getComputedStyle(c).transform : null;
        })(),
      };
    })()`);
    check(`[${vp.name}] 无横向溢出`, layout.overflowX <= 0, `overflowX=${layout.overflowX}`);
    check(
      `[${vp.name}] 设置按钮常显且够大`,
      layout.gearOpacity === "1" && layout.gearSize >= 28,
      `opacity=${layout.gearOpacity} size=${layout.gearSize}`
    );
    check(
      `[${vp.name}] 网格列数合理`,
      layout.gridCols >= 2 && layout.gridCols <= 6,
      `cols=${layout.gridCols}`
    );
    check(
      `[${vp.name}] 卡片无 hover 位移残留`,
      layout.coverLift === "none" || layout.coverLift === "matrix(1, 0, 0, 1, 0, 0)",
      `transform=${layout.coverLift}`
    );

    if (vp.full) {
      await goRoot();
      check(
        `[${vp.name}] 操作按钮占满一行（触屏目标大）`,
        Math.abs(layout.actionsWidth - (layout.innerWidth - 24)) < 2,
        `actions=${Math.round(layout.actionsWidth)} 期望=${layout.innerWidth - 24}`
      );

      /* ---------- 长按打开菜单 ---------- */
      const cards = await cardRects();
      if (cards.length === 0) {
        skip(`[${vp.name}] 长按/拖动/阅读器`, "根目录没有卡片");
        continue;
      }
      await longPress(cards[0].cx, cards[0].cy);
      await sleep(300);
      const menu = await evalJs(`(() => {
        const m = document.querySelector('.ctx-menu');
        if (!m) return null;
        const r = m.getBoundingClientRect();
        return { visible: true, inViewport: r.bottom <= window.innerHeight + 1 && r.right <= window.innerWidth + 1, items: m.querySelectorAll('.ctx-item').length };
      })()`);
      check(
        `[${vp.name}] 长按卡片打开操作菜单`,
        menu && menu.items >= 3 && menu.inViewport,
        menu ? `items=${menu.items} inViewport=${menu.inViewport}` : "无菜单"
      );
      const readerLeaked = await evalJs(`!!document.querySelector('.reader')`);
      check(`[${vp.name}] 长按未误触打开阅读器`, !readerLeaked);
      // 关闭菜单
      await tap(vp.width - 10, vp.height - 10);
      await sleep(200);

      /* ---------- 长按拖动排序（往返，落点只选漫画卡片，避免拖进文件夹） ---------- */
      const before = await cardOrder();
      const c = await cardRects();
      const mangaIdx = c.findIndex((k) => k.manga);
      if (c.length < 2 || mangaIdx < 0) {
        skip(`[${vp.name}] 长按拖动排序`, "卡片不足或没有漫画卡片可作为落点");
      } else {
        const dragIdx = mangaIdx === 0 ? 1 : 0;
        const target = c[mangaIdx];
        // 第一次：拖到漫画卡片右半侧 → 排到它后面
        await longPress(c[dragIdx].cx, c[dragIdx].cy, target.x + target.w - 6, target.cy);
        await sleep(700);
        const afterDrag = await cardOrder();
        const beforeArr = before.split(",");
        const afterArr = afterDrag.split(",");
        const rel = (arr, a, b) => arr.indexOf(a) - arr.indexOf(b);
        // 拖动项与目标项的相对次序应发生翻转
        const flipped =
          afterDrag !== before &&
          Math.sign(rel(afterArr, c[dragIdx].id, target.id)) ===
            -Math.sign(rel(beforeArr, c[dragIdx].id, target.id));
        check(
          `[${vp.name}] 长按拖动可排序`,
          flipped,
          `${before} -> ${afterDrag}`
        );
        const ghostGone = await evalJs(`!document.querySelector('.card-drag-ghost')`);
        check(`[${vp.name}] 拖动结束后浮动副本被清理`, ghostGone);

        // 第二次：拖回漫画卡片左半侧 → 排回它前面，还原顺序
        const c2 = await cardRects();
        const t2 = c2.find((k) => k.id === target.id);
        const d2 = c2.find((k) => k.id === c[dragIdx].id);
        if (t2 && d2) {
          await longPress(d2.cx, d2.cy, t2.x + 6, t2.cy);
          await sleep(700);
        }
        const restored = await cardOrder();
        check(`[${vp.name}] 往返拖动后顺序还原`, restored === before, `${restored}`);
      }

      /* ---------- 阅读器（触屏点按） ---------- */
      const manga = await evalJs(`(() => {
        const el = Array.from(document.querySelectorAll('.card-manga'))[0];
        if (!el) return null;
        const r = el.getBoundingClientRect();
        return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
      })()`);
      if (!manga) {
        skip(`[${vp.name}] 阅读器触屏检查`, "根目录没有漫画卡片");
      } else {
        await tap(manga.x, manga.y);
        let opened = false;
        for (let i = 0; i < 30; i += 1) {
          await sleep(300);
          opened = await evalJs(`!!document.querySelector('.reader')`);
          if (opened) break;
        }
        check(`[${vp.name}] 点按卡片打开阅读器`, opened);
        if (opened) {
          await sleep(1200);
          const rd = await evalJs(`(() => {
            const bar = document.querySelector('.reader-topbar');
            const slider = document.querySelector('.reader-slider');
            const zoom = document.querySelector('.reader-zoom');
            const img = document.querySelector('.reader-page');
            return {
              barTop: bar.getBoundingClientRect().top,
              sliderTop: slider.getBoundingClientRect().top,
              zoomBottom: zoom.getBoundingClientRect().bottom,
              sliderWidth: slider.getBoundingClientRect().width,
              innerWidth: window.innerWidth,
              imgWidth: img ? img.getBoundingClientRect().width : 0,
              winHeight: window.innerHeight,
            };
          })()`);
          check(
            `[${vp.name}] 顶栏两行排布（滑块换行到第二行）`,
            rd.sliderTop >= rd.zoomBottom - 1,
            `sliderTop=${Math.round(rd.sliderTop)} zoomBottom=${Math.round(rd.zoomBottom)}`
          );
          check(
            `[${vp.name}] 进度滑块占满一行`,
            rd.sliderWidth > rd.innerWidth * 0.7,
            `slider=${Math.round(rd.sliderWidth)} win=${rd.innerWidth}`
          );
          check(
            `[${vp.name}] 漫画图片宽度铺满屏幕`,
            Math.abs(rd.imgWidth - rd.innerWidth) < 2,
            `img=${Math.round(rd.imgWidth)} win=${rd.innerWidth}`
          );

          // 点按显隐 + 内容不移动
          const topBefore = await evalJs(
            `document.querySelector('.reader-page').getBoundingClientRect().top`
          );
          await tap(rd.innerWidth / 2, rd.winHeight * 0.6);
          await sleep(400);
          const hid = await evalJs(`(() => ({
            hidden: document.querySelector('.reader-topbar').classList.contains('chrome-hidden'),
            pageTop: document.querySelector('.reader-page').getBoundingClientRect().top,
          }))()`);
          check(`[${vp.name}] 点按画面隐藏工具栏`, hid.hidden);
          check(
            `[${vp.name}] 显隐不移动漫画内容`,
            Math.abs(hid.pageTop - topBefore) < 0.5,
            `${topBefore} -> ${hid.pageTop}`
          );
          await tap(rd.innerWidth / 2, rd.winHeight * 0.6);
          await sleep(300);

          // 退出
          await evalJs(`document.querySelector('.reader-topbar .btn').click()`);
          await sleep(400);
          check(`[${vp.name}] 可退出阅读器`, await evalJs(`!document.querySelector('.reader')`));
        }
      }

      /* ---------- 模态框底部面板 ---------- */
      const gear = await evalJs(`(() => {
        const el = document.querySelector('.card-settings');
        if (!el) return null;
        const r = el.getBoundingClientRect();
        return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
      })()`);
      if (gear) {
        await tap(gear.x, gear.y);
        await sleep(500);
        const modal = await evalJs(`(() => {
          const m = document.querySelector('.modal');
          if (!m) return null;
          const r = m.getBoundingClientRect();
          return { bottom: r.bottom, width: r.width, winHeight: window.innerHeight, winWidth: window.innerWidth };
        })()`);
        check(
          `[${vp.name}] 模态框为底部弹出面板`,
          modal && Math.abs(modal.bottom - modal.winHeight) < 2 && Math.abs(modal.width - modal.winWidth) < 2,
          modal ? `bottom=${Math.round(modal.bottom)}/${modal.winHeight} width=${Math.round(modal.width)}/${modal.winWidth}` : "无模态框"
        );
        await evalJs(`document.querySelector('.modal-close')?.click()`);
        await sleep(300);
      }
    }
  }

  const failed = results.filter((r) => !r.ok);
  console.log(`\n结果：${results.length - failed.length}/${results.length} 通过`);
  try {
    await send("Emulation.clearDeviceMetricsOverride");
    await send("Emulation.setTouchEmulationEnabled", { enabled: false });
  } catch {}
  ws.close();
  process.exit(failed.length ? 1 : 0);
}

main().catch((e) => {
  console.error("验证脚本异常：", e);
  process.exit(1);
});
