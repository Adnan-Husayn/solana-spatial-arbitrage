// Pricing and profit model, ported from the Rust crate (src/pricing.rs, src/orca.rs,
// src/math.rs, src/strategy.rs). tests/web/engine.test.mjs checks it against numbers
// produced by the Rust implementation, so keep the two in step.
//
// Token amounts are BigInt where exact integer math matters (Raydium), and Number for the
// Orca quotes, which the Rust code also computes in f64.

export const ADDRESSES = {
  rayCoinVault: "DQyrAcCrDXQ7NeoqGgDCZwBvWDcYmFCjSb9JtteuvPpz",
  rayPcVault: "HLmqeL62xR1QoZ1HKKbXRrdN1p3phKpxRMb2VVopvBBz",
  orcaWhirlpool: "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE",
};

const Q64 = 18446744073709551616; // 2^64
const SOL_DECIMALS = 9;
const USDC_DECIMALS = 6;
const RAY_FEE_NUM = 25n;
const RAY_FEE_DEN = 10000n;
const ORCA_FEE_DENOM = 1_000_000;
const U64_MAX = (1n << 64n) - 1n;

// Whirlpool account layout (8-byte Anchor discriminator first).
const ORCA_TICK_SPACING_OFFSET = 41;
const ORCA_FEE_RATE_OFFSET = 45;
const ORCA_LIQUIDITY_OFFSET = 49;
const ORCA_SQRT_PRICE_OFFSET = 65;
const ORCA_TICK_INDEX_OFFSET = 81;
const ORCA_MIN_LEN = ORCA_TICK_INDEX_OFFSET + 4;

function readUintLE(bytes, offset, length) {
  let v = 0n;
  for (let i = length - 1; i >= 0; i--) v = (v << 8n) | BigInt(bytes[offset + i]);
  return v;
}

/** SPL token account: amount is a u64 at bytes 64..72. */
export function tokenAmount(bytes) {
  if (bytes.length < 72) throw new Error(`token account too short: ${bytes.length}`);
  return readUintLE(bytes, 64, 8);
}

export function parseWhirlpool(bytes) {
  if (bytes.length < ORCA_MIN_LEN) throw new Error(`whirlpool account too short: ${bytes.length}`);
  return {
    sqrtPrice: readUintLE(bytes, ORCA_SQRT_PRICE_OFFSET, 16),
    tickIndex: Number(BigInt.asIntN(32, readUintLE(bytes, ORCA_TICK_INDEX_OFFSET, 4))),
    liquidity: readUintLE(bytes, ORCA_LIQUIDITY_OFFSET, 16),
    tickSpacing: Number(readUintLE(bytes, ORCA_TICK_SPACING_OFFSET, 2)),
    feeRate: Number(readUintLE(bytes, ORCA_FEE_RATE_OFFSET, 2)),
  };
}

/**
 * Market state. Shape:
 * { raySol: BigInt, rayUsdc: BigInt, orcaSqrtPrice: BigInt, orcaTickIndex: number,
 *   orcaLiquidity: BigInt, orcaTickSpacing: number, orcaFeeRate: number }
 */
export function isReady(st) {
  return st.raySol !== 0n && st.rayUsdc !== 0n && st.orcaSqrtPrice !== 0n;
}

// ---------- pricing (USDC per SOL) ----------

export function orcaPrice(sqrtPriceX64) {
  const s = Number(sqrtPriceX64) / Q64;
  return s * s * 10 ** (SOL_DECIMALS - USDC_DECIMALS);
}

export function raydiumPrice(solReserve, usdcReserve) {
  if (solReserve === 0n) return null;
  return Number(usdcReserve) / 10 ** USDC_DECIMALS / (Number(solReserve) / 10 ** SOL_DECIMALS);
}

export const Direction = Object.freeze({
  BuyRaydiumSellOrca: "BuyRaydiumSellOrca", // Orca is richer: sell SOL there, buy back on Raydium
  BuyOrcaSellRaydium: "BuyOrcaSellRaydium", // Raydium is richer: sell SOL there, buy back on Orca
});

export function spread(rayPrice, orcaPx) {
  if (!(rayPrice > 0) || !(orcaPx > 0) || !Number.isFinite(rayPrice) || !Number.isFinite(orcaPx)) return null;
  const orcaRicher = orcaPx >= rayPrice;
  const low = orcaRicher ? rayPrice : orcaPx;
  const high = orcaRicher ? orcaPx : rayPrice;
  return {
    direction: orcaRicher ? Direction.BuyRaydiumSellOrca : Direction.BuyOrcaSellRaydium,
    bps: ((high - low) / low) * 10_000,
    rayPrice,
    orcaPrice: orcaPx,
  };
}

/** Combined pool fees for one round trip, in basis points. */
export function feeHurdleBps(st) {
  return Number(RAY_FEE_NUM) / Number(RAY_FEE_DEN) * 10_000 + st.orcaFeeRate / 100;
}

// ---------- quotes ----------

/** Raydium constant-product output with the 0.25% fee. Exact integer math. Returns null on bad input. */
export function calculateSwapOut(amountIn, reserveIn, reserveOut) {
  if (amountIn === 0n) return 0n;
  if (reserveIn === 0n || reserveOut === 0n) return null;
  const effective = amountIn * (RAY_FEE_DEN - RAY_FEE_NUM);
  const out = (effective * reserveOut) / (reserveIn * RAY_FEE_DEN + effective);
  return out > U64_MAX ? null : out;
}

function sqrtAtTick(tick) {
  return Math.pow(1.0001, tick / 2);
}

/** Sqrt-price bounds of the tick-spacing-aligned range holding the current price. */
function orcaRange(st) {
  const spacing = st.orcaTickSpacing;
  if (spacing === 0) return null;
  const lower = Math.floor(st.orcaTickIndex / spacing) * spacing;
  return [sqrtAtTick(lower), sqrtAtTick(lower + spacing)];
}

function orcaUsable(st) {
  if (st.orcaLiquidity === 0n || st.orcaSqrtPrice === 0n) return null;
  return [Number(st.orcaSqrtPrice) / Q64, Number(st.orcaLiquidity)];
}

/** SOL in -> USDC out (raw units, Numbers). null if the swap would leave the current tick range. */
export function quoteSolToUsdc(st, solIn) {
  const u = orcaUsable(st);
  const r = orcaRange(st);
  if (!u || !r) return null;
  const [s, l] = u;
  const dx = solIn * (1 - st.orcaFeeRate / ORCA_FEE_DENOM);
  const sNew = (l * s) / (l + dx * s);
  if (sNew < r[0]) return null;
  return Math.floor(Math.max(l * (s - sNew), 0));
}

/** USDC in -> SOL out (raw units, Numbers). null if the swap would leave the current tick range. */
export function quoteUsdcToSol(st, usdcIn) {
  const u = orcaUsable(st);
  const r = orcaRange(st);
  if (!u || !r) return null;
  const [s, l] = u;
  const dy = usdcIn * (1 - st.orcaFeeRate / ORCA_FEE_DENOM);
  const sNew = s + dy / l;
  if (sNew > r[1]) return null;
  return Math.floor(Math.max(l * (1 / s - 1 / sNew), 0));
}

// ---------- strategy ----------

export const DEFAULT_CONFIG = Object.freeze({
  minTradeLamports: 10_000_000, // 0.01 SOL
  maxTradeLamports: 50_000_000_000, // 50 SOL
  tipLamports: 10_000,
  txFeeLamports: 25_000,
  minNetProfitLamports: 10_000,
});

export function fixedCost(cfg) {
  return cfg.tipLamports + cfg.txFeeLamports;
}

function rayOut(amountIn, reserveIn, reserveOut) {
  const out = calculateSwapOut(BigInt(amountIn), reserveIn, reserveOut);
  return out === null ? null : Number(out);
}

export function firstLegUsdc(st, direction, solIn) {
  return direction === Direction.BuyRaydiumSellOrca
    ? quoteSolToUsdc(st, solIn)
    : rayOut(solIn, st.raySol, st.rayUsdc);
}

export function secondLegSol(st, direction, usdc) {
  return direction === Direction.BuyRaydiumSellOrca
    ? rayOut(usdc, st.rayUsdc, st.raySol)
    : quoteUsdcToSol(st, usdc);
}

/** SOL out for a SOL-in round trip (lamports), or null if a leg can't be quoted. */
export function roundTrip(st, direction, solIn) {
  const usdc = firstLegUsdc(st, direction, solIn);
  return usdc === null ? null : secondLegSol(st, direction, usdc);
}

function gross(st, d, x) {
  const out = roundTrip(st, d, x);
  return out === null ? null : out - x;
}

/**
 * Best size for one direction: geometric grid, then a ternary refinement around the best point.
 * Mirrors `best_size` in src/strategy.rs, including its tie-breaking (last maximum wins).
 * Returns { size, gross } in lamports, or null.
 */
export function bestSize(st, d, cfg) {
  const GRID = 48;
  const min = Math.max(cfg.minTradeLamports, 1);
  const max = cfg.maxTradeLamports;
  if (min > max) return null;
  const ratio = Math.pow(max / min, 1 / GRID);
  const points = [];
  for (let i = 0; i <= GRID; i++) points.push(Math.round(min * Math.pow(ratio, i)));

  let idx = -1;
  let best = null;
  points.forEach((x, i) => {
    const g = gross(st, d, x);
    if (g !== null && (best === null || g >= best)) {
      best = g;
      idx = i;
    }
  });
  if (idx < 0) return null;

  let lo = points[Math.max(idx - 1, 0)];
  let hi = points[Math.min(idx + 1, points.length - 1)];
  while (hi - lo > 2) {
    const third = Math.floor((hi - lo) / 3);
    const m1 = lo + third;
    const m2 = hi - third;
    const g1 = gross(st, d, m1);
    const g2 = gross(st, d, m2);
    if (g1 !== null && g2 !== null && g1 < g2) lo = m1;
    else if (g1 !== null) hi = m2;
    else lo = m1;
  }
  let out = null;
  for (let x = lo; x <= hi; x++) {
    const g = gross(st, d, x);
    if (g !== null && (out === null || g >= out.gross)) out = { size: x, gross: g };
  }
  return out;
}

/** Best attempt in both directions, profitable or not. Used by the monitor to show the gap. */
export function bestAttempts(st, cfg = DEFAULT_CONFIG) {
  const cost = fixedCost(cfg);
  return [Direction.BuyRaydiumSellOrca, Direction.BuyOrcaSellRaydium].map((direction) => {
    const b = isReady(st) ? bestSize(st, direction, cfg) : null;
    if (!b) return { direction, quotable: false };
    const net = b.gross - cost;
    return {
      direction,
      quotable: true,
      amountIn: b.size,
      expectedOut: b.size + b.gross,
      grossProfit: b.gross,
      cost,
      netProfit: net,
      profitable: net >= cfg.minNetProfitLamports,
    };
  });
}

/** The most profitable opportunity above the configured threshold, or null. */
export function evaluate(st, cfg = DEFAULT_CONFIG) {
  if (!isReady(st)) return null;
  let best = null;
  for (const a of bestAttempts(st, cfg)) {
    if (a.quotable && a.profitable && (best === null || a.netProfit >= best.netProfit)) best = a;
  }
  return best;
}
