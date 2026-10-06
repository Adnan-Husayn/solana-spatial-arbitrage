# Solana Spatial Arbitrage Bot

A Rust bot that watches the SOL/USDC pair on **Raydium V4** (constant-product AMM) and **Orca Whirlpools** (concentrated liquidity), works out whether a round trip between them is profitable after fees, and builds the atomic two-swap transaction to capture it.

It runs in **simulation mode by default**. Live sending through Jito exists, is off unless you turn it on, and has not been run with real funds. See [Status](#status).

## How it works

1. **Listen.** Account updates for the Raydium vaults and the Orca pool arrive over WebSocket (`accountSubscribe`) and are written into a shared `MarketState`. The listener reconnects with backoff if the socket drops.
2. **Trigger.** A `logsSubscribe` on the Raydium pool wakes the main loop. It evaluates once per slot and skips if the state is stale.
3. **Price.** Spot prices come from Raydium reserves and Orca's `sqrt_price`. Quotes use the real swap math: Raydium's constant-product formula and a closed-form Orca quote within the current tick range.
4. **Decide.** Both directions are evaluated. The trade size that maximizes profit is found by search, then the Jito tip and transaction fees are subtracted. Only opportunities above a threshold go further.
5. **Build.** One v0 transaction: `[ComputeBudget, Swap A, Swap B, Jito tip]` (about 1.2 KB, so no lookup table is needed). The second swap's minimum output is the input plus costs, so the whole transaction reverts on-chain unless it ends in SOL profit.
6. **Execute.** In simulation mode the transaction is simulated against mainnet and logged. In live mode it is checked against the risk limits, simulated again, sent as a Jito bundle, and its outcome and realized PnL are tracked.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the design in more detail.

## Status

| Area | State |
|---|---|
| Pricing, quotes, profit model | Done. Orca quotes match an on-chain swap to within a few lamports. |
| Orca swap instruction | Done. Simulated successfully against mainnet (`sim_orca`). |
| Raydium swap instruction | Built, but **not yet validated** against mainnet with a funded wallet. |
| Combined transaction | Built and size-checked. Needs a funded wallet to simulate to success. |
| Live execution | Written and unit-tested; **never run with real funds**. |

Known limits:

- Orca quotes only cover the current tick range (about 0.04% of price). Larger trades are rejected rather than estimated. Crossing ticks needs tick-array reads.
- Raydium prices use vault balances, which differ slightly from the pool's true reserves.
- This pair is heavily contested. On public WebSockets, expect to lose most races, and expect most of the time to show no profitable opportunity. At the time of writing, the spread was around 10 bps against roughly 29 bps of combined pool fees.

## Live monitor

`docs/` holds a static, read-only web page that shows the live spread, the fee hurdle and the best trade in each direction. It polls public Solana RPCs from the browser and runs a JavaScript port of the pricing and profit model. It never touches a wallet.

```bash
python3 -m http.server 8000 --directory docs
```

Then open <http://localhost:8000>. The port is checked against output from the Rust implementation:

```bash
node --test tests/web/engine.test.mjs
```

## Getting started

### Prerequisites

- [Rust](https://rustup.rs/) (edition 2024, so a recent stable toolchain).
- A Solana mainnet RPC with WebSocket support. [Helius](https://www.helius.dev/) works well; the free tier is enough to try it. Public endpoints rate-limit quickly.

### Setup

```bash
git clone https://github.com/Adnan-Husayn/solana-spatial-arbitrage.git
cd solana-spatial-arbitrage
```

Create a `.env` file (it is git-ignored):

```env
RPC_URL=https://mainnet.helius-rpc.com/?api-key=YOUR_API_KEY
PRIVATE_KEY=your_private_key_as_base58_or_json_byte_array
```

Use a **dedicated, throwaway wallet**. Never put a key that holds real funds here until you have read the code and tested in simulation.

```bash
cargo build --release
```

### Run

```bash
cargo run --release --bin spatial_arbitrage_bot
```

Expected output in simulation mode:

```text
INFO bot active (simulation mode, set LIVE=true to send)
INFO wallet: <your pubkey>
INFO openbook market keys fetched
INFO bootstrap complete
INFO listening for raydium pool activity
INFO ray 121.1507  orca 121.0367  spread 9.42 bps | slots 1 opportunities 0 simulated 0 ok 0 sent 0 landed 0
```

When an opportunity appears you'll see an `opportunity` line, then the result of simulating it. With an unfunded wallet the simulation is rejected for missing token accounts. That is expected, and it does **not** mean the swap logic is proven correct.

## Configuration

Only `RPC_URL` and `PRIVATE_KEY` are required. Everything else is an optional environment variable (or `.env` entry).

| Variable | Default | Meaning |
|---|---|---|
| `LIVE` | `false` | Send real bundles. Off means simulate only. |
| `MAX_LIVE_TRADE_SOL` | `1` | Hard cap on trade size when live |
| `MAX_DAILY_LOSS_SOL` | `0.1` | Stop trading for the UTC day once realized losses reach this |
| `MIN_BALANCE_SOL` | `0.05` | Native SOL that must stay in the wallet |
| `KILL_FILE` | `KILL` | Trading halts while this file exists |
| `JITO_URL` | Amsterdam block engine | Jito bundles endpoint |
| `MIN_TRADE_SOL` / `MAX_TRADE_SOL` | `0.01` / `50` | Strategy search range |
| `TIP_LAMPORTS` / `TX_FEE_LAMPORTS` | `10000` / `25000` | Per-trade costs used in the profit model |
| `MIN_NET_PROFIT_LAMPORTS` | `10000` | Ignore opportunities below this |
| `RUST_LOG` | `info` | Log level |

### Going live

Live mode needs a wallet with both a WSOL and a USDC associated token account, and some wrapped SOL. Before setting `LIVE=true`:

1. Run in simulation mode with that wallet and confirm simulations succeed.
2. Fund it only with an amount you can afford to lose.
3. Keep the limits above conservative. To halt immediately, create the kill file (`touch KILL`).

## Development

```bash
cargo test --lib      # unit tests
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

CI runs the same three checks, plus the JavaScript engine tests.

Helper binaries in `src/bin`:

| Binary | Purpose |
|---|---|
| `fetch_keys` | Prints the Raydium, OpenBook and Orca account keys for the configured pools |
| `sim_orca` | Simulates an Orca swap against mainnet with signature checks off. Nothing is sent and no wallet is needed. |
| `test_connection`, `test_logs` | Quick RPC and WebSocket connectivity checks |
| `test_math` | Interactive check of the Raydium swap formula against a real transaction |

Source layout:

| Module | Role |
|---|---|
| `config` | Addresses and account layout offsets (single source of truth) |
| `state` | Shared market state and raw account parsing |
| `listener` | WebSocket ingestion with reconnect |
| `pricing` | Spot prices and spread |
| `orca`, `math` | Orca and Raydium swap quotes |
| `strategy` | Profit model and trade size search |
| `instructions` | Raydium and Orca swap instruction builders |
| `executor` | Transaction building, simulation, wallet balances |
| `risk` | Live-trading limits and kill switch |
| `jito` | Bundle submission and status polling |

## Disclaimer

For educational purposes. The software is provided as is, with no warranty. Arbitrage and MEV are competitive and risky, and failed or unprofitable transactions can lose money. Do not use it with a wallet holding significant funds.
