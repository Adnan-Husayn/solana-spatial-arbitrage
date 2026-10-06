import {
  ADDRESSES, DEFAULT_CONFIG, Direction, bestAttempts, feeHurdleBps, firstLegUsdc, isReady, orcaPrice,
  parseWhirlpool, raydiumPrice, spread, tokenAmount,
} from "./engine.js";

// Public endpoints that allow browser (CORS) requests. Tried in order; rotated on failure.
const PUBLIC_ENDPOINTS = ["https://solana-rpc.publicnode.com", "https://public.rpc.solanavibestation.com"];
const POLL_MS = 4000;
const STALE_MS = 30_000;
const MAX_SAMPLES = 900; // one hour at the poll rate
const MIN_WINDOW_MS = 120_000;
const STORAGE_KEY = "arb-monitor-rpc";
const SVG_NS = "http://www.w3.org/2000/svg";

const $ = (id) => document.getElementById(id);
const samples = [];
let endpoints = PUBLIC_ENDPOINTS.slice();
let endpointIndex = 0;
let lastSuccess = 0;
let lastError = null;
let hoverIndex = null;
let inFlight = false;

// ---------- storage (best effort: private windows may throw) ----------

function loadCustomRpc() {
  try {
    return localStorage.getItem(STORAGE_KEY) || "";
  } catch {
    return "";
  }
}

function saveCustomRpc(url) {
  try {
    if (url) localStorage.setItem(STORAGE_KEY, url);
    else localStorage.removeItem(STORAGE_KEY);
  } catch {
    /* not persisted; the setting still applies to this page load */
  }
}

function applyCustomRpc(url) {
  endpoints = url ? [url, ...PUBLIC_ENDPOINTS] : PUBLIC_ENDPOINTS.slice();
  endpointIndex = 0;
}

// ---------- data ----------

function decodeBase64(b64) {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

async function fetchState(endpoint) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 8000);
  try {
    const res = await fetch(endpoint, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        method: "getMultipleAccounts",
        params: [
          [ADDRESSES.rayCoinVault, ADDRESSES.rayPcVault, ADDRESSES.orcaWhirlpool],
          { encoding: "base64", commitment: "confirmed" },
        ],
      }),
      signal: controller.signal,
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const json = await res.json();
    if (json.error) throw new Error(json.error.message || "RPC error");
    const value = json.result?.value;
    if (!Array.isArray(value) || value.length !== 3 || value.some((v) => !v)) throw new Error("accounts missing");
    const [sol, usdc, pool] = value.map((v) => decodeBase64(v.data[0]));
    const w = parseWhirlpool(pool);
    return {
      slot: json.result.context.slot,
      state: {
        raySol: tokenAmount(sol),
        rayUsdc: tokenAmount(usdc),
        orcaSqrtPrice: w.sqrtPrice,
        orcaTickIndex: w.tickIndex,
        orcaLiquidity: w.liquidity,
        orcaTickSpacing: w.tickSpacing,
        orcaFeeRate: w.feeRate,
      },
    };
  } finally {
    clearTimeout(timeout);
  }
}

async function poll() {
  if (inFlight || document.hidden) return;
  inFlight = true;
  try {
    const { slot, state } = await fetchState(endpoints[endpointIndex]);
    lastSuccess = Date.now();
    lastError = null;
    if (isReady(state)) {
      const s = spread(raydiumPrice(state.raySol, state.rayUsdc), orcaPrice(state.orcaSqrtPrice));
      const last = samples[samples.length - 1];
      if (s && (!last || slot > last.slot)) {
        samples.push({
          t: lastSuccess,
          slot,
          ray: s.rayPrice,
          orca: s.orcaPrice,
          bps: s.bps,
          direction: s.direction,
          hurdle: feeHurdleBps(state),
          attempts: bestAttempts(state, DEFAULT_CONFIG).map((a) =>
            a.quotable ? { ...a, midUsdc: firstLegUsdc(state, a.direction, a.amountIn) } : a,
          ),
        });
        if (samples.length > MAX_SAMPLES) samples.shift();
      }
    }
  } catch (e) {
    lastError = e?.name === "AbortError" ? "request timed out" : String(e?.message || e);
    endpointIndex = (endpointIndex + 1) % endpoints.length;
  } finally {
    inFlight = false;
    render();
  }
}

// ---------- formatting ----------

const fmtPrice = (v) => `$${v.toLocaleString("en-US", { minimumFractionDigits: 4, maximumFractionDigits: 4 })}`;
const fmtBps = (v) => v.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
const fmtTime = (t) => new Date(t).toLocaleTimeString("en-GB", { hour12: false });
const fmtSol = (lamports, digits = 4) =>
  (lamports / 1e9).toLocaleString("en-US", { minimumFractionDigits: digits, maximumFractionDigits: digits });
const fmtSignedSol = (lamports, digits = 6) => {
  const sign = lamports > 0 ? "+" : lamports < 0 ? "−" : "";
  return `${sign}${fmtSol(Math.abs(lamports), digits)}`;
};
const fmtUsdc = (raw) =>
  (raw / 1e6).toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
const ROUTES = {
  [Direction.BuyRaydiumSellOrca]: { title: "Sell on Orca, buy back on Raydium", first: "Orca", second: "Raydium" },
  [Direction.BuyOrcaSellRaydium]: { title: "Sell on Raydium, buy back on Orca", first: "Raydium", second: "Orca" },
};

function hostOf(url) {
  try {
    return new URL(url).host;
  } catch {
    return "custom endpoint";
  }
}

// ---------- rendering ----------

function renderStatus() {
  const pill = $("status");
  const text = $("status-text");
  const last = samples[samples.length - 1];
  const age = lastSuccess ? Date.now() - lastSuccess : Infinity;
  if (document.hidden) {
    pill.dataset.state = "idle";
    text.textContent = "Paused while this tab is in the background";
  } else if (!lastSuccess) {
    pill.dataset.state = lastError ? "error" : "idle";
    text.textContent = lastError ? `Can't reach an RPC (${lastError}). Retrying…` : "Connecting…";
  } else if (age > STALE_MS) {
    pill.dataset.state = "stale";
    text.textContent = `Stale · last update ${Math.round(age / 1000)}s ago · retrying`;
  } else {
    pill.dataset.state = "live";
    text.textContent = `Live · slot ${last ? last.slot.toLocaleString("en-US") : "–"} · ${hostOf(endpoints[endpointIndex])}`;
  }
}

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function setTag(id, text) {
  const tag = $(id);
  tag.hidden = !text;
  tag.textContent = text || "";
}

function renderHeadline() {
  const last = samples[samples.length - 1];
  if (!last) return;

  $("hero-bps").textContent = fmtBps(last.bps);
  $("hero-figure").dataset.empty = "false";
  for (const id of ["gauge-mark", "gauge-zone", "gauge-mark-label"]) $(id).hidden = false;
  $("price-ray").textContent = fmtPrice(last.ray);
  $("price-orca").textContent = fmtPrice(last.orca);
  const orcaHigher = last.orca >= last.ray;
  setTag("tag-orca", orcaHigher ? "Higher" : "Lower");
  setTag("tag-ray", orcaHigher ? "Lower" : "Higher");

  // Gauge: spread against the fee hurdle, on a scale that always leaves room past break-even.
  const scaleMax = Math.max(last.hurdle * 1.5, last.bps * 1.15);
  const markPct = (last.hurdle / scaleMax) * 100;
  $("gauge-fill").style.width = `${(last.bps / scaleMax) * 100}%`;
  $("gauge-mark").style.left = `${markPct}%`;
  $("gauge-zone").style.left = `${markPct}%`;
  const markLabel = $("gauge-mark-label");
  markLabel.style.left = `${markPct}%`;
  markLabel.textContent = `Break-even ${fmtBps(last.hurdle)}`;
  const share = last.hurdle > 0 ? last.bps / last.hurdle : 0;
  $("gauge").setAttribute("aria-valuenow", String(Math.min(Math.round(share * 100), 100)));
  const gap = last.hurdle - last.bps;
  $("gauge-value").textContent = gap > 0 ? `${fmtBps(gap)} bps short` : `${fmtBps(-gap)} bps past`;
  $("gauge-caption").textContent =
    gap > 0
      ? `The spread covers ${Math.round(share * 100)}% of the pool fees a round trip pays. Below break-even, no trade size can profit.`
      : "The spread now covers the pool fees. Price impact, the tip and the transaction fee still decide whether a trade profits.";

  const best = last.attempts
    .filter((a) => a.quotable)
    .reduce((m, a) => (m === null || a.netProfit > m.netProfit ? a : m), null);
  const verdict = $("verdict");
  if (best && best.profitable) {
    verdict.dataset.state = "good";
    $("verdict-icon").textContent = "✓";
    $("verdict-text").textContent = `Profitable in the model: ${fmtSignedSol(best.netProfit)} SOL`;
  } else {
    verdict.dataset.state = "none";
    $("verdict-icon").textContent = "–";
    $("verdict-text").textContent = "No profitable trade right now";
  }

  const lo = Math.min(...samples.map((s) => s.bps));
  const hi = Math.max(...samples.map((s) => s.bps));
  $("range").textContent = `${fmtBps(lo)} – ${fmtBps(hi)} bps`;
}

function ledgerRow(label, value, className) {
  const row = el("div", className);
  row.append(el("dt", null, label), el("dd", null, value));
  return row;
}

function renderRoute(a) {
  const route = ROUTES[a.direction];
  const card = el("article", "panel route");
  const head = el("div", "route-head");
  head.append(el("h3", "route-title", route.title));
  card.append(head);

  if (!a.quotable) {
    card.append(el("p", "hint", "This route cannot be quoted inside Orca's current tick range."));
    return card;
  }

  const result = el("span", "result");
  result.dataset.state = a.profitable ? "good" : "none";
  const icon = el("span", "result-icon", a.profitable ? "✓" : "–");
  icon.setAttribute("aria-hidden", "true");
  result.append(icon, el("span", null, a.profitable ? "Profitable" : "Loses money"));
  head.append(result);

  const flow = el("ol", "flow");
  const step = (label, amount) => {
    const li = el("li");
    li.append(el("span", "flow-step", label), el("span", "flow-amount", amount));
    return li;
  };
  flow.append(
    step("Start with", `${fmtSol(a.amountIn)} SOL`),
    step(`Sell on ${route.first}`, a.midUsdc === null ? "–" : `${fmtUsdc(a.midUsdc)} USDC`),
    step(`Buy back on ${route.second}`, `${fmtSol(a.expectedOut, 6)} SOL`),
  );
  card.append(flow);

  const ledger = el("dl", "ledger");
  ledger.append(
    ledgerRow("Gross", `${fmtSignedSol(a.grossProfit)} SOL`),
    ledgerRow("Tip and transaction fee", `−${fmtSol(a.cost, 6)} SOL`),
    ledgerRow("Net", `${fmtSignedSol(a.netProfit)} SOL`, "total"),
  );
  card.append(ledger);
  return card;
}

function renderRoutes() {
  const last = samples[samples.length - 1];
  if (!last) return;
  $("routes").replaceChildren(...last.attempts.map(renderRoute));
}

function cell(text, className) {
  const td = document.createElement("td");
  td.textContent = text;
  if (className) td.className = className;
  return td;
}

function renderSamplesTable() {
  const rows = samples.slice(-30).reverse();
  $("samples-body").replaceChildren(
    ...rows.map((s) => {
      const tr = document.createElement("tr");
      tr.append(
        cell(fmtTime(s.t)),
        cell(s.slot.toLocaleString("en-US")),
        cell(fmtPrice(s.ray)),
        cell(fmtPrice(s.orca)),
        cell(fmtBps(s.bps)),
      );
      return tr;
    }),
  );
}

// ---------- chart ----------

function svg(name, attrs = {}, text) {
  const node = document.createElementNS(SVG_NS, name);
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, v);
  if (text !== undefined) node.textContent = text;
  return node;
}

function niceCeil(v) {
  const steps = [1, 2, 4, 5, 8, 10]; // each divides into 4 clean ticks
  const mag = Math.pow(10, Math.floor(Math.log10(v)));
  const hit = steps.find((s) => s * mag >= v);
  return (hit ?? 10) * mag;
}

let chartGeom = null;

function renderChart() {
  const chart = $("chart");
  const width = chart.clientWidth || 600;
  const height = chart.clientHeight || 300;
  chart.setAttribute("viewBox", `0 0 ${width} ${height}`);
  $("chart-empty").hidden = samples.length > 0;
  if (samples.length === 0) {
    chart.replaceChildren();
    chartGeom = null;
    return;
  }

  const m = { top: 16, right: 64, bottom: 28, left: 40 };
  const pw = Math.max(width - m.left - m.right, 10);
  const ph = Math.max(height - m.top - m.bottom, 10);

  const last = samples[samples.length - 1];
  const hurdle = last.hurdle;
  const tMax = last.t;
  const tMin = Math.min(samples[0].t, tMax - MIN_WINDOW_MS);
  const yTop = niceCeil(Math.max(hurdle, ...samples.map((s) => s.bps)) * 1.15);
  const x = (t) => m.left + ((t - tMin) / (tMax - tMin)) * pw;
  const y = (v) => m.top + ph - (v / yTop) * ph;

  const nodes = [];

  // y grid + ticks
  const TICKS = 4;
  for (let i = 0; i <= TICKS; i++) {
    const v = (yTop / TICKS) * i;
    nodes.push(svg("line", { class: i === 0 ? "baseline" : "grid", x1: m.left, x2: m.left + pw, y1: y(v), y2: y(v) }));
    nodes.push(svg("text", { x: m.left - 8, y: y(v) + 4, "text-anchor": "end" }, String(Number(v.toFixed(2)))));
  }

  // x ticks
  const xTicks = pw < 420 ? 1 : 3; // just the ends on narrow screens, so labels never collide
  for (let i = 0; i <= xTicks; i++) {
    const t = tMin + ((tMax - tMin) / xTicks) * i;
    const anchor = i === 0 ? "start" : i === xTicks ? "end" : "middle";
    nodes.push(svg("text", { x: x(t), y: m.top + ph + 18, "text-anchor": anchor }, fmtTime(t)));
  }

  // fee hurdle reference line, labelled directly; above it, pool fees are covered
  nodes.unshift(svg("rect", { class: "zone", x: m.left, y: m.top, width: pw, height: Math.max(y(hurdle) - m.top, 0) }));
  nodes.push(svg("line", { class: "hurdle", x1: m.left, x2: m.left + pw, y1: y(hurdle), y2: y(hurdle) }));
  nodes.push(svg("text", { class: "hurdle-label", x: m.left + 6, y: y(hurdle) + 16 }, `Fee hurdle ${fmtBps(hurdle)} bps`));
  if (y(hurdle) - m.top > 22) {
    nodes.push(svg("text", { class: "zone-label", x: m.left + 6, y: y(hurdle) - 8 }, "Fees covered above this line"));
  }

  // series
  const pts = samples.map((s) => [x(s.t), y(s.bps)]);
  const path = pts.map(([px, py], i) => `${i ? "L" : "M"}${px.toFixed(1)},${py.toFixed(1)}`).join(" ");
  const base = y(0);
  nodes.push(svg("path", { class: "area", d: `${path} L${pts[pts.length - 1][0].toFixed(1)},${base} L${pts[0][0].toFixed(1)},${base} Z` }));
  nodes.push(svg("path", { class: "line", d: path }));

  // crosshair for the hovered / focused sample
  const hi = hoverIndex !== null && samples[hoverIndex] ? hoverIndex : null;
  if (hi !== null) {
    nodes.push(svg("line", { class: "crosshair", x1: pts[hi][0], x2: pts[hi][0], y1: m.top, y2: m.top + ph }));
    nodes.push(svg("circle", { class: "dot", cx: pts[hi][0], cy: pts[hi][1], r: 5 }));
  }

  // end marker and its value
  const [ex, ey] = pts[pts.length - 1];
  nodes.push(svg("circle", { class: "dot", cx: ex, cy: ey, r: 5 }));
  nodes.push(svg("text", { class: "end-label", x: ex + 10, y: ey + 4 }, fmtBps(last.bps)));

  chart.replaceChildren(...nodes);
  chartGeom = { pts, m, pw, width };
  renderTooltip();
}

function renderTooltip() {
  const tip = $("tooltip");
  if (hoverIndex === null || !chartGeom || !samples[hoverIndex]) {
    tip.hidden = true;
    return;
  }
  const s = samples[hoverIndex];
  const row = (name, value, keyed) => {
    const div = document.createElement("div");
    div.className = "tt-row";
    const label = document.createElement("span");
    label.className = "tt-name";
    if (keyed) {
      const key = document.createElement("span");
      key.className = "tt-key";
      label.append(key);
    }
    label.append(document.createTextNode(name));
    const strong = document.createElement("strong");
    strong.textContent = value;
    div.append(strong, label);
    div.style.flexDirection = "row-reverse";
    return div;
  };
  const time = document.createElement("div");
  time.className = "tt-time";
  time.textContent = `${fmtTime(s.t)} · slot ${s.slot.toLocaleString("en-US")}`;
  tip.replaceChildren(
    time,
    row("Spread", `${fmtBps(s.bps)} bps`, true),
    row("Raydium", fmtPrice(s.ray), false),
    row("Orca", fmtPrice(s.orca), false),
  );
  tip.hidden = false;
  const px = chartGeom.pts[hoverIndex][0];
  const w = tip.offsetWidth;
  const left = px + 12 + w > chartGeom.width ? px - 12 - w : px + 12;
  tip.style.left = `${Math.max(left, 0)}px`;
}

function nearestIndex(clientX) {
  if (!chartGeom) return null;
  const rect = $("chart").getBoundingClientRect();
  const px = clientX - rect.left;
  let best = 0;
  let bestDist = Infinity;
  chartGeom.pts.forEach(([x], i) => {
    const d = Math.abs(x - px);
    if (d < bestDist) {
      bestDist = d;
      best = i;
    }
  });
  return best;
}

function setHover(index) {
  if (index === hoverIndex) return;
  hoverIndex = index;
  renderChart();
}

function render() {
  renderStatus();
  renderHeadline();
  renderRoutes();
  renderSamplesTable();
  renderChart();
}

// ---------- wiring ----------

function start() {
  const custom = loadCustomRpc();
  applyCustomRpc(custom);
  $("rpc-input").value = custom;

  const chart = $("chart");
  chart.addEventListener("pointermove", (e) => setHover(nearestIndex(e.clientX)));
  chart.addEventListener("pointerleave", () => setHover(null));
  chart.addEventListener("focus", () => {
    if (samples.length && hoverIndex === null) setHover(samples.length - 1);
  });
  chart.addEventListener("blur", () => setHover(null));
  chart.addEventListener("keydown", (e) => {
    if (!samples.length) return;
    const cur = hoverIndex ?? samples.length - 1;
    const next =
      e.key === "ArrowLeft" ? cur - 1
      : e.key === "ArrowRight" ? cur + 1
      : e.key === "Home" ? 0
      : e.key === "End" ? samples.length - 1
      : null;
    if (next === null) return;
    e.preventDefault();
    setHover(Math.min(Math.max(next, 0), samples.length - 1));
  });

  $("rpc-form").addEventListener("submit", (e) => {
    e.preventDefault();
    const url = $("rpc-input").value.trim();
    if (url && !/^https:\/\//i.test(url)) {
      $("rpc-input").setCustomValidity("Use an https:// URL");
      $("rpc-input").reportValidity();
      return;
    }
    $("rpc-input").setCustomValidity("");
    saveCustomRpc(url);
    applyCustomRpc(url);
    poll();
  });
  $("rpc-reset").addEventListener("click", () => {
    $("rpc-input").value = "";
    $("rpc-input").setCustomValidity("");
    saveCustomRpc("");
    applyCustomRpc("");
    poll();
  });

  new ResizeObserver(() => renderChart()).observe($("chart-wrap"));
  document.addEventListener("visibilitychange", () => {
    renderStatus();
    if (!document.hidden) poll();
  });

  render();
  poll();
  setInterval(poll, POLL_MS);
  setInterval(renderStatus, 1000);
}

start();
