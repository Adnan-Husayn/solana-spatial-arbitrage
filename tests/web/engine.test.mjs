// Cross-checks the JavaScript engine against values produced by the Rust implementation.
// Run with: node --test tests/web
import test from "node:test";
import assert from "node:assert/strict";
import {
  Direction, DEFAULT_CONFIG, bestSize, calculateSwapOut, evaluate, feeHurdleBps, orcaPrice, parseWhirlpool,
  quoteSolToUsdc, quoteUsdcToSol, raydiumPrice, roundTrip, spread, tokenAmount,
} from "../../docs/engine.js";

// Mainnet snapshot, 2026-10-06 (same fixture as src/orca.rs).
const snapshot = () => ({
  raySol: 116_647_489_467_425n,
  rayUsdc: 14_158_027_040_682n,
  orcaSqrtPrice: 6_425_431_916_669_708_712n,
  orcaTickIndex: -21094,
  orcaLiquidity: 1_090_735_051_258_027n,
  orcaTickSpacing: 4,
  orcaFeeRate: 400,
});

// Each case: Orca state after shifting the price, and what the Rust code returned for it.
const RUST_CASES = [
  { sqrt: 6425431916669708288n, tick: -21094, s2u: 121280594, u2s: 823874243,
    sellOrca: [10000633, -32716, 996719987], sellRay: [10000940, -25271, 997464466], evalNet: null },
  { sqrt: 6457479157451097088n, tick: -20995, s2u: 122493399, u2s: 815717073,
    sellOrca: [49999999488, 312288243, 1006687093], sellRay: [10000569, -124035, 987588582],
    evalNet: [Direction.BuyRaydiumSellOrca, 49999999488, 312253243] },
  { sqrt: 6393224035070270464n, tick: -21195, s2u: 120067788, u2s: 832196204,
    sellOrca: [10000340, -132391, 986752871], sellRay: [49999999712, 355103718, 1007539863],
    evalNet: [Direction.BuyOrcaSellRaydium, 49999999712, 355068718] },
  { sqrt: 6441475467029006336n, tick: -21044, s2u: 121886997, u2s: 819775367,
    sellOrca: [49999991270, 63326391, 1001703545], sellRay: [10000560, -74898, 992501957],
    evalNet: [Direction.BuyRaydiumSellOrca, 49999991270, 63291391] },
  { sqrt: 6414177554617007104n, tick: -21129, s2u: 120856112, u2s: 826767930,
    sellOrca: [10000567, -67603, 993231498], sellRay: [49999965017, 26648555, 1000967853],
    evalNet: [Direction.BuyOrcaSellRaydium, 49999965017, 26613555] },
];

test("spot prices match the Rust fixtures", () => {
  assert.ok(Math.abs(orcaPrice(6_429_939_587_537_051_150n) - 121.4995) < 0.01);
  assert.ok(Math.abs(raydiumPrice(116_647_489_467_425n, 14_158_027_040_682n) - 121.37) < 0.05);
  assert.ok(Math.abs(orcaPrice(1n << 64n) - 1000) < 1e-9);
  assert.equal(raydiumPrice(0n, 100n), null);
});

test("spread direction and size", () => {
  const a = spread(100, 101);
  assert.equal(a.direction, Direction.BuyRaydiumSellOrca);
  assert.ok(Math.abs(a.bps - 100) < 1e-9);
  assert.equal(spread(101, 100).direction, Direction.BuyOrcaSellRaydium);
  assert.equal(spread(0, 100), null);
  assert.equal(spread(NaN, 100), null);
});

test("fee hurdle is Raydium 25 bps plus the Orca fee tier", () => {
  assert.equal(feeHurdleBps(snapshot()), 29);
});

test("Raydium swap math is exact", () => {
  // out = in*9975*rOut / (rIn*10000 + in*9975)
  assert.equal(calculateSwapOut(1_000_000n, 1_000_000_000n, 2_000_000_000n), 1_993_011n);
  assert.equal(calculateSwapOut(0n, 1n, 1n), 0n);
  assert.equal(calculateSwapOut(1n, 0n, 1n), null);
});

test("Orca quote matches an on-chain swap", () => {
  // Simulated mainnet swap: 100 USDC in -> 825_622_708 lamports out.
  const st = { ...snapshot(), orcaSqrtPrice: 6_418_624_578_571_055_275n };
  assert.ok(Math.abs(quoteUsdcToSol(st, 100_000_000) - 825_622_708) <= 10);
});

test("Orca quotes reject swaps that leave the tick range", () => {
  assert.equal(quoteSolToUsdc(snapshot(), 1_000_000 * 1e9), null);
  assert.equal(quoteUsdcToSol(snapshot(), 1e19), null);
  assert.equal(quoteSolToUsdc({ ...snapshot(), orcaLiquidity: 0n }, 1e9), null);
});

test("quotes, round trips, best sizes and decisions equal the Rust output", () => {
  for (const c of RUST_CASES) {
    const st = { ...snapshot(), orcaSqrtPrice: c.sqrt, orcaTickIndex: c.tick };
    assert.equal(quoteSolToUsdc(st, 1_000_000_000), c.s2u);
    assert.equal(quoteUsdcToSol(st, 100_000_000), c.u2s);
    for (const [dir, exp] of [[Direction.BuyRaydiumSellOrca, c.sellOrca], [Direction.BuyOrcaSellRaydium, c.sellRay]]) {
      const b = bestSize(st, dir, DEFAULT_CONFIG);
      assert.deepEqual([b.size, b.gross], [exp[0], exp[1]], `best size ${dir} tick ${c.tick}`);
      assert.equal(roundTrip(st, dir, 1_000_000_000), exp[2]);
    }
    const e = evaluate(st, DEFAULT_CONFIG);
    if (c.evalNet === null) assert.equal(e, null);
    else assert.deepEqual([e.direction, e.amountIn, e.netProfit], c.evalNet);
  }
});

test("account parsing", () => {
  const pool = new Uint8Array(653);
  const put = (off, value, len) => { for (let i = 0; i < len; i++) pool[off + i] = Number((value >> BigInt(8 * i)) & 0xffn); };
  put(41, 4n, 2); put(45, 400n, 2); put(49, 1_090_735_051_258_027n, 16);
  put(65, 6_429_939_587_537_051_150n, 16); put(81, BigInt.asUintN(32, -21080n), 4);
  assert.deepEqual(parseWhirlpool(pool), {
    sqrtPrice: 6_429_939_587_537_051_150n, tickIndex: -21080, liquidity: 1_090_735_051_258_027n, tickSpacing: 4, feeRate: 400,
  });
  assert.throws(() => parseWhirlpool(new Uint8Array(50)));

  const tok = new Uint8Array(165);
  tok.set([0x21, 0x43, 0x65, 0x87, 0, 0, 0, 0], 64);
  assert.equal(tokenAmount(tok), 0x87654321n);
  assert.throws(() => tokenAmount(new Uint8Array(10)));
});
