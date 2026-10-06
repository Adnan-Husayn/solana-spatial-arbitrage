# Architecture

## Data flow

```text
                  Solana mainnet
   ┌──────────────────────────────────────────┐
   │ Raydium V4 vaults   Orca Whirlpool pool  │
   └───────┬──────────────────────┬───────────┘
           │ accountSubscribe     │ logsSubscribe (Raydium pool)
           ▼                      ▼
   ┌───────────────┐      ┌───────────────────┐
   │ listener task │      │    main loop      │
   │ (reconnects)  │      │ once per slot:    │
   └───────┬───────┘      │  snapshot state   │
           │ write        │  evaluate         │
           ▼              │  risk check       │
   ┌───────────────┐ read │  build, simulate  │
   │ MarketState   │◄─────┤  send (if live)   │
   │ Arc<RwLock>   │      └─────────┬─────────┘
   └───────────────┘                │ bundle
                                    ▼
                              Jito block engine
```

## Concurrency model

Two independent streams do different jobs:

- **State feed.** A background task subscribes to account updates for the two Raydium vaults and the Orca pool. It decodes the raw bytes and writes them into `MarketState` behind an `RwLock`. If the socket drops it reconnects with exponential backoff.
- **Trigger feed.** The main loop subscribes to logs mentioning the Raydium pool. Activity there is the cue to re-evaluate. It processes at most one event per slot.

The main loop copies the state out with `snapshot()` and drops the lock immediately, so no lock is ever held across an `.await`. If no account update has arrived recently, the state is treated as stale and the loop skips.

Pricing and the profit search are pure functions of a `MarketState` value. They do no I/O, which is why they are tested against fixed mainnet snapshots.

## Decision pipeline

1. **Quote.** Raydium uses the constant-product formula with its 0.25% fee. Orca uses a closed-form quote assuming constant liquidity inside the current tick range; a swap that would leave that range is rejected, because liquidity beyond it depends on tick arrays the bot does not read yet.
2. **Search.** Every cycle starts and ends in SOL. For each direction (sell on Orca and buy on Raydium, or the reverse), a geometric grid over trade sizes finds the best region, then a ternary search refines it.
3. **Cost.** The Jito tip and transaction fees are subtracted. Opportunities below `MIN_NET_PROFIT_LAMPORTS` are dropped.
4. **Build.** `[ComputeBudget, Swap A, Swap B, Tip]` as a v0 transaction. The second swap's minimum output equals the input plus costs, so the transaction cannot succeed unless it ends in profit.
5. **Guard.** In live mode the trade is re-sized to the wallet's real WSOL balance and checked against the per-trade cap, the daily loss cap, the minimum balance and the kill switch. It is then simulated; a failing simulation is never sent.
6. **Send and settle.** The transaction goes out as a Jito bundle. Status is polled, and realized PnL is measured from the change in total wallet value.

## Design decisions

| Decision | Reasoning |
|---|---|
| **Rust** | No garbage collector pauses on the hot path, and the type system catches layout and unit mistakes. |
| **WebSocket push over polling** | `getAccountInfo` polling hits rate limits and adds a round trip per read. The cost is needing reconnect logic. |
| **Both swaps in one transaction** | If the second leg fails, the first reverts, so there is no inventory risk from getting stuck on one side. |
| **Profit enforced on-chain** | The second leg's minimum output makes unprofitable execution impossible even if the local quote was stale. |
| **Jito bundles** | A bundle only lands if it succeeds, and the tip competes for inclusion. A bundle that doesn't land costs nothing. |
| **Conservative Orca quotes** | Rejecting out-of-range trades is safer than guessing liquidity across ticks. |
| **Live mode off by default** | Real sending requires `LIVE=true`, and every trade passes the risk checks. |

## Limitations and next steps

- **Tick crossing.** Read the neighbouring tick arrays so larger Orca trades can be quoted accurately.
- **Pool reserves.** Raydium's true reserves also depend on open orders and owed fees, not only the vault balances.
- **Validation.** The Raydium swap instruction still needs a simulation against mainnet with a funded wallet.
- **Latency.** Public WebSockets are slower than a co-located Geyser/gRPC stream. For a pair this contested, that decides who wins.
- **Wider strategy.** Other pools or triangular routes would face less competition than SOL/USDC.
