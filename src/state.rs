use crate::config::*;
use anyhow::{Result, anyhow};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, Default)]
pub struct MarketState {
    pub ray_sol: u64,
    pub ray_usdc: u64,
    pub orca_sqrt_price: u128,
    pub orca_tick_index: i32,
    pub last_update: u64,
}

pub type SharedState = Arc<RwLock<MarketState>>;

pub fn new_shared() -> SharedState {
    Arc::new(RwLock::new(MarketState::default()))
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl MarketState {
    pub fn is_ready(&self) -> bool {
        self.ray_sol != 0 && self.ray_usdc != 0 && self.orca_sqrt_price != 0
    }
}

/// Reads (sqrt_price Q64.64, current tick index) from raw Whirlpool account data.
pub fn parse_whirlpool(data: &[u8]) -> Result<(u128, i32)> {
    if data.len() < ORCA_MIN_LEN {
        return Err(anyhow!("whirlpool account too short: {}", data.len()));
    }
    let sqrt = u128::from_le_bytes(data[ORCA_SQRT_PRICE_OFFSET..ORCA_SQRT_PRICE_OFFSET + 16].try_into()?);
    let tick = i32::from_le_bytes(data[ORCA_TICK_INDEX_OFFSET..ORCA_TICK_INDEX_OFFSET + 4].try_into()?);
    Ok((sqrt, tick))
}

pub fn set_orca(state: &SharedState, data: &[u8]) -> Result<()> {
    let (sqrt, tick) = parse_whirlpool(data)?;
    let mut w = state.write().map_err(|_| anyhow!("state lock poisoned"))?;
    w.orca_sqrt_price = sqrt;
    w.orca_tick_index = tick;
    w.last_update = now_secs();
    Ok(())
}

pub fn set_ray_sol(state: &SharedState, amount: u64) -> Result<()> {
    let mut w = state.write().map_err(|_| anyhow!("state lock poisoned"))?;
    w.ray_sol = amount;
    w.last_update = now_secs();
    Ok(())
}

pub fn set_ray_usdc(state: &SharedState, amount: u64) -> Result<()> {
    let mut w = state.write().map_err(|_| anyhow!("state lock poisoned"))?;
    w.ray_usdc = amount;
    w.last_update = now_secs();
    Ok(())
}

/// Copy the state out so callers never hold the lock across `.await`.
pub fn snapshot(state: &SharedState) -> Result<MarketState> {
    state.read().map(|g| *g).map_err(|_| anyhow!("state lock poisoned"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_whirlpool(sqrt: u128, tick: i32) -> Vec<u8> {
        let mut d = vec![0u8; 653];
        d[ORCA_SQRT_PRICE_OFFSET..ORCA_SQRT_PRICE_OFFSET + 16].copy_from_slice(&sqrt.to_le_bytes());
        d[ORCA_TICK_INDEX_OFFSET..ORCA_TICK_INDEX_OFFSET + 4].copy_from_slice(&tick.to_le_bytes());
        d
    }

    #[test]
    fn parses_whirlpool_fields() {
        let (s, t) = parse_whirlpool(&fake_whirlpool(6_429_939_587_537_051_150, -21080)).unwrap();
        assert_eq!(s, 6_429_939_587_537_051_150);
        assert_eq!(t, -21080);
    }

    #[test]
    fn short_account_errors() {
        assert!(parse_whirlpool(&[0u8; 50]).is_err());
    }

    #[test]
    fn setters_and_snapshot() {
        let st = new_shared();
        assert!(!snapshot(&st).unwrap().is_ready());
        set_ray_sol(&st, 1).unwrap();
        set_ray_usdc(&st, 2).unwrap();
        set_orca(&st, &fake_whirlpool(5, 3)).unwrap();
        let s = snapshot(&st).unwrap();
        assert!(s.is_ready());
        assert_eq!((s.ray_sol, s.ray_usdc, s.orca_tick_index), (1, 2, 3));
    }
}
