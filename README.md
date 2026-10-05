
---

# 🦀 Solana Spatial Arbitrage Bot (Rust)

> **High-Frequency Trading (HFT) Engine for Raydium & Orca**

A high-performance arbitrage bot written in **Rust** that monitors liquidity pools on the Solana blockchain in real-time. It detects price discrepancies between **Raydium V4** (AMM) and **Orca Whirlpools** (CLMM), calculates profitability (net of fees), and simulates atomic swap transactions against Mainnet state.

---

## ⚡ Key Features

* **Low Latency:** Built with Rust for microsecond-level execution logic.
* **Async Stream Architecture:** Uses **Tokio** and **WebSockets (PubSub)** to stream account updates (PUSH) rather than polling (PULL), minimizing reaction time.
* **Thread-Safe State:** Implements `Arc<RwLock<MarketState>>` for safe, concurrent read/write access to live market data across threads.
* **Dual Protocol Support:**
* **Raydium V4:** Standard Constant Product AMM integration with dynamic OpenBook key fetching.
* **Orca Whirlpools:** Concentrated Liquidity (CLMM) integration with Tick Array logic.


* **Simulation Engine:** Features a "Dry Run" mode that builds actual transaction bundles and simulates them against the live blockchain to verify validity without risking funds.

## 🛠 Tech Stack

* **Language:** Rust 🦀
* **Blockchain:** Solana (Mainnet Beta)
* **RPC/WSS Provider:** Helius (for high-performance Geyser plugins)
* **Execution:** Jito Block Engine (Bundle support integrated)
* **Serialization:** Bincode & Borsh

---

## 🏗 Architecture

The bot operates in three concurrent phases:

1. **The Listener (Background Threads):**
* Connects to Helius WSS.
* Subscribes to `accountSubscribe` for specific Pool Vaults (SOL/USDC).
* Updates the shared `MarketState` in memory whenever a slot changes.


2. **The Calculator (Main Loop):**
* Listens for transaction logs (to trigger checks immediately after a trade happens).
* Calculates the spread: `(Orca Price - Raydium Price) / Raydium Price`.
* Factors in fees (0.25% Raydium + 0.04% Orca).


3. **The Executor:**
* Builds a Versioned Transaction containing: `[Compute Budget] -> [Swap A] -> [Swap B] -> [Jito Tip]`.
* Simulates the transaction via `simulateTransaction` to ensure success before broadcasting.



---

## 🚀 Getting Started

### Prerequisites

* [Rust & Cargo](https://rustup.rs/) installed.
* [Solana CLI](https://docs.solana.com/cli/install-solana-cli-tools) installed.
* A **Helius API Key** (Free tier works for testing).

### Installation

1. **Clone the Repository**
```bash
git clone https://github.com/Adnan-Husayn/solana-spatial-arbitrage.git
cd solana-spatial-arbitrage

```


2. **Configure Environment**
Create a `.env` file in the root directory:
```bash
touch .env

```


Add your credentials:
```env
# Your Helius RPC/WSS URL
RPC_URL=https://mainnet.helius-rpc.com/?api-key=YOUR_API_KEY

# Your Solana Wallet Private Key (Base58 String)
PRIVATE_KEY=your_private_key_base58_here

```


3. **Build the Project**
```bash
cargo build --release

```



---

## Configuration

All settings are optional environment variables (or `.env` entries) besides `RPC_URL` and `PRIVATE_KEY`.

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

Live mode needs a dedicated wallet with a WSOL and a USDC token account. Keep only an amount you can afford to lose in it.

## 🏃 Usage

Run the bot in **Simulation Mode** (Default). This will monitor the chain and attempt to simulate a trade whenever it detects activity.

```bash
cargo run --bin spatial_arbitrage_bot

```

### Expected Output

If your environment is set up correctly, you will see:

```text
 Bot Active (SIMULATION MODE)
   Wallet: 9mnov...
   My WSOL ATA: 4L9GC...
   My USDC ATA: DJqbW...
 Connecting to Network...
 Fetching Active OpenBook Market Data...
    Market Keys Fetched Successfully
 Bootstrapping Liquidity...
 Bootstrap Complete.
 Listening for Logs (SIMULATION MODE)...

 Simulating Trade against Mainnet...
 Simulation Completed with Expected Funds Error: AccountNotFound
    SUCCESS: The bot logic and keys are 100% correct.

```

> **Note:** The "AccountNotFound" or "Insufficient Funds" error during simulation is **SUCCESS**. It confirms that the transaction was built correctly, the keys were valid, and the logic executed until it tried to withdraw funds from your empty test wallet.

---

## 📜 Disclaimer

**EDUCATIONAL PURPOSE ONLY.**
This software is provided "as is" with no warranty. High-Frequency Trading (HFT) and MEV (Maximal Extractable Value) are highly competitive and risky fields.

* Do not use this with a wallet containing significant funds until you have extensively tested it.
* Slippage, network latency, and failed transactions can result in financial loss.

---