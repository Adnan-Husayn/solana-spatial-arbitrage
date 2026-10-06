import {
  ADDRESSES, DEFAULT_CONFIG, Direction, bestAttempts, feeHurdleBps, isReady, orcaPrice, parseWhirlpool,
  raydiumPrice, spread, tokenAmount,
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
          attempts: bestAttempts(state, DEFAULT_CONFIG),
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
const ROUTE_LABEL = {
  [Direction.BuyRaydiumSellOrca]: "Sell on Orca, buy back on Raydium",
  [Direction.BuyOrcaSellRaydium]: "Sell on Raydium, buy back on Orca",
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
  const el = $("status");
  const text = $("status-text");
  const last = samples[samples.length - 1];
  const age = lastSuccess ? Date.now() - lastSuccess : Infinity;
  if (document.hidden) {
    el.dataset.state = "idle";
    text.textContent = "Paused while this tab is in the background";
  } else if (!lastSuccess) {
    el.dataset.state = lastError ? "error" : "idle";
    text.textContent = lastError ? `Can't reach an RPC (${lastError}). Retrying…` : "Connecting…";
  } else if (age > STALE_MS) {
    el.dataset.state = "stale";
    text.textContent = `Stale · last update ${Math.round(age / 1000)}s ago · retrying`;
  } else {
    el.dataset.state = "live";
    text.textContent = `Live · slot ${last ? last.slot.toLocaleString("en-US") : "–"} · ${hostOf(endpoints[endpointIndex])}`;
  }
}

function renderHeadline() {
  const last = samples[samples.length - 1];
  if (!last) return;

  $("hero-bps").textContent = fmtBps(last.bps);
  $("kpi-ray").textContent = fmtPrice(last.ray);
  $("kpi-orca").textContent = fmtPrice(last.orca);
  $("kpi-hurdle").textContent = `${fmtBps(last.hurdle)} bps`;

  const share = last.hurdle > 0 ? last.bps / last.hurdle : 0;
  const pct = Math.round(share * 100);
  const fill = $("meter-fill");
  fill.style.width = `${Math.min(share, 1) * 100}%`;
  fill.dataset.full = String(share >= 1);
  $("meter").setAttribute("aria-valuenow", String(Math.min(pct, 100)));
  $("meter-caption").textContent =
    share >= 1
      ? `The spread is ${pct}% of the ${fmtBps(last.hurdle)} bps fee hurdle, so pool fees are covered. Price impact and costs still apply.`
      : `The spread covers ${pct}% of the ${fmtBps(last.hurdle)} bps fee hurdle. It has to pass 100% before any size can profit.`;

  const quotable = last.attempts.filter((a) => a.quotable);
  const best = quotable.reduce((m, a) => (m === null || a.netProfit > m.netProfit ? a : m), null);
  const verdict = $("verdict");
  if (best && best.profitable) {
    verdict.dataset.state = "good";
    $("verdict-icon").textContent = "✓";
    $("verdict-text").textContent = `Profitable in the model: ${fmtSignedSol(best.netProfit)} SOL net`;
  } else {
    verdict.dataset.state = "none";
    $("verdict-icon").textContent = "–";
    $("verdict-text").textContent = "No profitable trade right now";
  }

  if (best) {
    $("kpi-net").textContent = `${fmtSignedSol(best.netProfit)} SOL`;
    $("kpi-net-sub").textContent = `${ROUTE_LABEL[best.direction]}, ${fmtSol(best.amountIn)} SOL in`;
  } else {
    $("kpi-net").textContent = "–";
    $("kpi-net-sub").textContent = "No route can be quoted right now";
  }
}

function cell(text, className) {
  const td = document.createElement("td");
  td.textContent = text;
  if (className) td.className = className;
  return td;
}

function renderTrades() {
  const last = samples[samples.length - 1];
  if (!last) return;
  const body = $("trades-body");
  body.replaceChildren(
    ...last.attempts.map((a) => {
      const tr = document.createElement("tr");
      tr.append(cell(ROUTE_LABEL[a.direction]));
      if (!a.quotable) {
        const td = cell("Cannot be quoted inside the current Orca tick range", "muted");
        td.colSpan = 5;
        td.style.textAlign = "left";
        tr.append(td);
        return tr;
      }
      tr.append(
        cell(fmtSol(a.amountIn)),
        cell(fmtSignedSol(a.grossProfit)),
        cell(fmtSol(a.cost, 6)),
        cell(fmtSignedSol(a.netProfit)),
      );
      const td = document.createElement("td");
      const wrap = document.createElement("span");
      wrap.className = "result";
      wrap.dataset.state = a.profitable ? "good" : "none";
      const icon = document.createElement("span");
      icon.className = "result-icon";
      icon.setAttribute("aria-hidden", "true");
      icon.textContent = a.profitable ? "✓" : "–";
      const label = document.createElement("span");
      label.textContent = a.profitable ? "Profitable" : "Loses money";
      wrap.append(icon, label);
      td.append(wrap);
      tr.append(td);
      return tr;
    }),
  );
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
  const el = document.createElementNS(SVG_NS, name);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  if (text !== undefined) el.textContent = text;
  return el;
}

function niceCeil(v) {
  const steps = [1, 2, 4, 5, 8, 10]; // each divides into 4 clean ticks
  const mag = Math.pow(10, Math.floor(Math.log10(v)));
  const hit = steps.find((s) => s * mag >= v);
  return (hit ?? 10) * mag;
}

let chartGeom = null;

function renderChart() {
  const el = $("chart");
  const width = el.clientWidth || 600;
  const height = el.clientHeight || 280;
  el.setAttribute("viewBox", `0 0 ${width} ${height}`);
  $("chart-empty").hidden = samples.length > 0;
  if (samples.length === 0) {
    el.replaceChildren();
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
  for (let i = 0; i <= 3; i++) {
    const t = tMin + ((tMax - tMin) / 3) * i;
    const anchor = i === 0 ? "start" : i === 3 ? "end" : "middle";
    nodes.push(svg("text", { x: x(t), y: m.top + ph + 18, "text-anchor": anchor }, fmtTime(t)));
  }

  // fee hurdle reference line, labelled directly
  nodes.push(svg("line", { class: "hurdle", x1: m.left, x2: m.left + pw, y1: y(hurdle), y2: y(hurdle) }));
  nodes.push(svg("text", { class: "hurdle-label", x: m.left + 4, y: y(hurdle) - 6 }, `Fee hurdle ${fmtBps(hurdle)} bps`));

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

  el.replaceChildren(...nodes);
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
  renderTrades();
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
