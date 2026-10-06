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
          orcaFeeBps: state.orcaFeeRate / 100,
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
    text.textContent = lastError ? `Can't reach an RPC (${lastError}) · retrying` : "Connecting…";
  } else if (age > STALE_MS) {
    pill.dataset.state = "stale";
    text.textContent = `Stale · last update ${Math.round(age / 1000)}s ago · retrying`;
  } else {
    pill.dataset.state = "live";
    const slot = last ? last.slot.toLocaleString("en-US") : "–";
    text.textContent = `Live · slot ${slot} · ${fmtTime(lastSuccess)} · ${hostOf(endpoints[endpointIndex])}`;
  }
}

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

/** Builds a sentence from plain strings and [text] arrays, which are set in emphasis. */
function setSentence(target, parts) {
  target.replaceChildren(
    ...parts.map((part) => (Array.isArray(part) ? el("i", null, part[0]) : document.createTextNode(part))),
  );
}

const fmtFee = (bps) => String(Number(bps.toFixed(2)));

function bestAttempt(sample) {
  return sample.attempts
    .filter((a) => a.quotable)
    .reduce((m, a) => (m === null || a.netProfit > m.netProfit ? a : m), null);
}

function renderHeadline() {
  const last = samples[samples.length - 1];
  if (!last) return;

  $("hero-bps").textContent = fmtBps(last.bps);
  const gap = last.hurdle - last.bps;
  const best = bestAttempt(last);
  const hurdleText = fmtFee(last.hurdle);
  if (best && best.profitable) {
    setSentence($("deck"), [
      "That clears the ", [hurdleText], " the pools charge. The model finds a trade worth ",
      [`${fmtSignedSol(best.netProfit)} SOL`], ".",
    ]);
  } else if (gap > 0) {
    setSentence($("deck"), [
      "It costs ", [hurdleText], " to trade the gap, so there is nothing to do. The spread is ",
      [`${fmtBps(gap)} short`], " of break-even.",
    ]);
  } else {
    setSentence($("deck"), [
      "That clears the ", [hurdleText], " the pools charge, but price impact, the tip and the transaction fee still leave ",
      ["no profit"], ".",
    ]);
  }

  const rayHigher = last.ray > last.orca;
  $("price-ray").textContent = fmtPrice(last.ray);
  $("price-orca").textContent = fmtPrice(last.orca);
  $("note-ray").textContent = `${rayHigher ? "Higher" : "Lower"} · from vault balances`;
  $("note-orca").textContent = `${rayHigher ? "Lower" : "Higher"} · from the sqrt price`;
  $("hurdle").textContent = fmtBps(last.hurdle);
  $("note-hurdle").textContent = `25 Raydium + ${fmtFee(last.orcaFeeBps)} Orca`;
  $("covered").textContent = last.hurdle > 0 ? `${Math.round((last.bps / last.hurdle) * 100)}%` : "–";

  const lo = Math.min(...samples.map((s) => s.bps));
  const hi = Math.max(...samples.map((s) => s.bps));
  $("range").textContent = `BASIS POINTS · RANGE ${fmtBps(lo)} – ${fmtBps(hi)}`;

  const dear = rayHigher ? "Raydium" : "Orca";
  const cheap = rayHigher ? "Orca" : "Raydium";
  $("essay-dear").textContent =
    `${dear} is the dearer venue right now. A SOL fetches ${fmtPrice(Math.max(last.ray, last.orca))} there and ` +
    `${fmtPrice(Math.min(last.ray, last.orca))} on ${cheap}. In principle you sell where it is dear and buy back where it is cheap.`;
  $("essay-hurdle").textContent =
    `The round trip pays both pools: Raydium takes 25 basis points and this Orca pool takes ${fmtFee(last.orcaFeeBps)}. ` +
    "Past that, a Jito tip, a transaction fee and price impact still have to be covered.";
}

function renderRoute(a, isBest) {
  const route = ROUTES[a.direction];
  const card = el("article", "route");
  const top = el("div", "route-top");
  top.append(el("h3", null, route.title));
  card.append(top);

  if (!a.quotable) {
    card.append(el("p", "parts", "Cannot be quoted inside Orca's current tick range."));
    return card;
  }

  card.dataset.state = a.profitable ? "good" : "none";
  top.append(el("p", "net", fmtSignedSol(a.netProfit)));
  const mid = a.midUsdc === null ? "–" : `${fmtUsdc(a.midUsdc)} USDC`;
  card.append(el("p", "path", `${fmtSol(a.amountIn)} SOL → ${mid} → ${fmtSol(a.expectedOut, 6)} SOL`));

  const outcome = a.profitable ? "a profit in the model" : isBest ? "the better way, still a loss" : "a loss";
  const parts = el("p", "parts");
  parts.append(
    "Gross ", el("span", "fig-mono", fmtSignedSol(a.grossProfit)),
    " · tip and fee ", el("span", "fig-mono", `−${fmtSol(a.cost, 6)}`),
    " · ", el("span", "outcome", outcome),
  );
  card.append(parts);
  return card;
}

function renderRoutes() {
  const last = samples[samples.length - 1];
  if (!last) return;
  const best = bestAttempt(last);
  const bothQuotable = last.attempts.every((a) => a.quotable);
  $("routes").replaceChildren(...last.attempts.map((a) => renderRoute(a, bothQuotable && a === best)));
}

function cell(text) {
  const td = document.createElement("td");
  td.textContent = text;
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

  const m = { top: 10, right: 64, bottom: 24, left: 28 };
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
  const note = `Above ${fmtFee(hurdle)}, the pools' fees are covered`;
  const noteY = y(hurdle) - m.top > 26 ? y(hurdle) - 10 : y(hurdle) + 20;
  nodes.push(svg("text", { class: "zone-label", x: m.left + 10, y: noteY }, pw < 300 ? `Fee hurdle ${fmtFee(hurdle)}` : note));

  // series
  const pts = samples.map((s) => [x(s.t), y(s.bps)]);
  const path = pts.map(([px, py], i) => `${i ? "L" : "M"}${px.toFixed(1)},${py.toFixed(1)}`).join(" ");
  nodes.push(svg("path", { class: "line", d: path }));

  // crosshair for the hovered / focused sample
  const hi = hoverIndex !== null && samples[hoverIndex] ? hoverIndex : null;
  if (hi !== null) {
    nodes.push(svg("line", { class: "crosshair", x1: pts[hi][0], x2: pts[hi][0], y1: m.top, y2: m.top + ph }));
    nodes.push(svg("circle", { class: "dot", cx: pts[hi][0], cy: pts[hi][1], r: 5.5 }));
  }

  // end marker and its value
  const [ex, ey] = pts[pts.length - 1];
  nodes.push(svg("circle", { class: "dot", cx: ex, cy: ey, r: 5.5 }));
  nodes.push(svg("text", { class: "end-label", x: ex + 12, y: ey + 7 }, fmtBps(last.bps)));

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
  const row = (name, value, className) => {
    const div = el("div", className ? `tt-row ${className}` : "tt-row");
    div.append(el("span", "tt-name", name), el("strong", null, value));
    return div;
  };
  tip.replaceChildren(
    el("div", "tt-time", `${fmtTime(s.t)} · SLOT ${s.slot.toLocaleString("en-US")}`),
    row("Spread", `${fmtBps(s.bps)} bps`, "lead-row"),
    row("Raydium", fmtPrice(s.ray)),
    row("Orca", fmtPrice(s.orca)),
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
