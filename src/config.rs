//! Single source of truth for on-chain addresses and account layout offsets.

pub const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
pub const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

pub const SOL_DECIMALS: u32 = 9;
pub const USDC_DECIMALS: u32 = 6;

// Raydium V4 SOL/USDC pool
pub const RAY_POOL: &str = "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2";
pub const RAY_AUTH: &str = "5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1";
pub const RAY_OPEN_ORDERS: &str = "HmiHHzq4Fym9e1D4qzLS6LDDM3tNsCTBPDWHTLZ763jY";
pub const RAY_TARGET_ORDERS: &str = "CZza3Ej4Mc58MnxWA385itCC9jCo3L1D7zc3LKy1bZMR";
pub const RAY_COIN_VAULT: &str = "DQyrAcCrDXQ7NeoqGgDCZwBvWDcYmFCjSb9JtteuvPpz";
pub const RAY_PC_VAULT: &str = "HLmqeL62xR1QoZ1HKKbXRrdN1p3phKpxRMb2VVopvBBz";

// OpenBook market the Raydium pool is bound to (AmmInfo.serum_market)
pub const OB_PROG_ID: &str = "srmqPvymJeFKQ4zGQed1GFppgkRHL9kaELCbyksJtPX";
pub const OB_MARKET_ID: &str = "8BnEgHoWFysVcuFFX7QztDmzuH8r5ZFvyP3sYwn1XTh6";

// Orca Whirlpool SOL/USDC (token A = SOL, token B = USDC)
pub const ORCA_WHIRLPOOL: &str = "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE";

// Whirlpool account layout (Anchor: 8-byte discriminator first)
pub const ORCA_TICK_SPACING_OFFSET: usize = 41;
pub const ORCA_FEE_RATE_OFFSET: usize = 45;
pub const ORCA_LIQUIDITY_OFFSET: usize = 49;
pub const ORCA_SQRT_PRICE_OFFSET: usize = 65;
pub const ORCA_TICK_INDEX_OFFSET: usize = 81;
pub const ORCA_MIN_LEN: usize = ORCA_TICK_INDEX_OFFSET + 4;

// Raydium AmmInfo: pool vault pubkeys start here
pub const RAY_AMM_KEYS_OFFSET: usize = 336;
