
---

### **1. The System Architecture (ASCII Diagram)**

Visuals prove you understand the data flow. Add this to the top of your README.

```mermaid
graph TD
    subgraph Blockchain Layer
        RPC[Helius RPC]
        WSS[Helius WSS]
        Jito[Jito Block Engine]
    end

    subgraph "Spatial Arb Bot (Rust)"
        Listener[Async Listener\n(Tokio Task)]
        State[Shared Market State\n(Arc<RwLock>)]
        Logic[Profit Engine\n(Main Thread)]
        Builder[Tx Builder]
    end

    WSS -- "Account Updates (Push)" --> Listener
    Listener -- "Write New Prices" --> State
    
    WSS -- "Log Notifications" --> Logic
    Logic -- "Read Prices" --> State
    
    Logic -- "Profit > Threshold?" --> Builder
    Builder -- "Simulate/Send Bundle" --> RPC & Jito

```

```text
+---------------------+           +-------------------------+
|    SOLANA MAINNET   |           |    SPATIAL ARB BOT      |
|                     |   WSS     |                         |
|  [Raydium V4 Pool]  +---------> |  [Async Listener Task]  |
|  [Orca Whirlpool]   |  (Push)   |           |             |
|                     |           |           v (Write)     |
+---------------------+           |  [Shared Market State]  |
                                  |     (Arc<RwLock>)       |
+---------------------+           |           ^ (Read)      |
|     EXECUTION       |           |           |             |
|                     |  <------+ |    [Profit Engine]      |
|  [Jito Block Engine]|   Bundle  | (Triggered by Logs)     |
+---------------------+           +-------------------------+

```

---

### **2. Architecture Explanation (The "How")**

*Add this section to demonstrate systems engineering knowledge.*

**The "Dual-Eye" Concurrency Model**
Instead of a simple linear loop, this bot employs a concurrent actor model to minimize latency:

* **The Left Eye (State Manager):** A dedicated background thread subscribes to `accountSubscribe` feeds. It continuously deserializes raw bytes from Raydium (AMM) and Orca (CLMM) into a shared memory state (`Arc<RwLock>`). This ensures the bot always has the absolute latest price in memory (0-latency access).
* **The Right Eye (Trigger Engine):** The main thread listens for `logsSubscribe`. It idles until a transaction occurs in the pool (a "Maker" trade), which serves as the signal to wake up.
* **Zero-Copy Logic:** By decoupling "fetching" from "calculating," the critical path (Calculation -> Execution) involves **zero network calls**. It reads from local memory and builds the transaction immediately.

---

### **3. Engineering Trade-offs (The "Why")**

*Recruiters love this. It shows you made conscious decisions, not just copied code.*

| Decision | Trade-off Analysis |
| --- | --- |
| **Rust vs. Python/JS** | **Why Rust?** Arbitrage is a "winner-takes-all" game. Rust provides zero-cost abstractions and memory safety without a Garbage Collector (GC). A JS/Python GC pause of 50ms is enough to lose an arbitrage opportunity to a competitor. |
| **Push (WSS) vs. Pull (HTTP)** | **Why WSS?** Polling (`getAccountInfo`) hits rate limits and introduces network round-trip latency. WebSockets push updates instantly as blocks are processed. **Downside:** Requires robust error handling for connection drops (reconnection logic). |
| **Jito vs. Standard RPC** | **Why Jito?** Standard transactions often fail or land slowly during congestion. Jito Bundles allow us to bribe validators directly for inclusion and offer "revert protection" (the bot doesn't pay gas if the arb fails). |
| **Atomic vs. Statistical** | **Why Atomic?** We bundle both swaps (Buy + Sell) into one transaction. If the second leg fails, the entire transaction reverts. This eliminates "Inventory Risk" (getting stuck holding a token you can't sell). |

---

### **4. "What I Would Do With More Time" (Future Roadmap)**

*This shows ambition and honesty about the project's current limits.*

1. **Geyser Plugin Integration:** Currently, we use WSS (Public Internet). The next step is deploying this binary directly onto a Solana RPC node using a **Geyser Plugin**, reducing latency from ~50-100ms (Network) to ~1ms (Local RAM).
2. **Flash Loan Integration:** Integrate **Solend** or **Drift** flash loans to borrow capital for the trade. This allows leveraging infinite liquidity without holding $100k in the wallet.
3. **Graph-Based Pathfinding:** Move beyond "Spatial Arb" (A -> B -> A) to "Triangular Arb" (SOL -> USDC -> RAY -> SOL). This requires a Directed Acyclic Graph (DAG) algorithm like Bellman-Ford optimized for DeFi.
4. **Hardware Optimization:** Pin specific threads to specific CPU cores (`core_affinity`) to prevent context-switching overhead during high-load events.

---