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
    pub orca_liquidity: u128,
    pub orca_tick_spacing: u16,
    /// Hundredths of a basis point (400 = 0.04%).
    pub orca_fee_rate: u16,
    pub last_update: u64,
}

/// SPL token account layout: mint [0..32], owner [32..64], amount u64 [64..72].
pub fn token_amount(data: &[u8]) -> Result<u64> {
    if data.len() < 72 {
        return Err(anyhow!("token account too short: {}", data.len()));
    }
    Ok(u64::from_le_bytes(data[64..72].try_into()?))
}

pub fn token_owner(data: &[u8]) -> Result<solana_sdk::pubkey::Pubkey> {
    if data.len() < 64 {
        return Err(anyhow!("token account too short: {}", data.len()));
    }
    Ok(solana_sdk::pubkey::Pubkey::new_from_array(
        data[32..64].try_into()?,
    ))
}

pub type SharedState = Arc<RwLock<MarketState>>;

pub fn new_shared() -> SharedState {
    Arc::new(RwLock::new(MarketState::default()))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl MarketState {
    /// True if no account update has arrived within `max_age_secs`.
    pub fn is_stale(&self, max_age_secs: u64) -> bool {
        now_secs().saturating_sub(self.last_update) > max_age_secs
    }

    pub fn is_ready(&self) -> bool {
        self.ray_sol != 0 && self.ray_usdc != 0 && self.orca_sqrt_price != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Whirlpool {
    pub sqrt_price: u128,
    pub tick_index: i32,
    pub liquidity: u128,
    pub tick_spacing: u16,
    pub fee_rate: u16,
}

/// Parses the fields we need from raw Whirlpool account data.
pub fn parse_whirlpool(data: &[u8]) -> Result<Whirlpool> {
    if data.len() < ORCA_MIN_LEN {
        return Err(anyhow!("whirlpool account too short: {}", data.len()));
    }
    Ok(Whirlpool {
        sqrt_price: u128::from_le_bytes(
            data[ORCA_SQRT_PRICE_OFFSET..ORCA_SQRT_PRICE_OFFSET + 16].try_into()?,
        ),
        tick_index: i32::from_le_bytes(
            data[ORCA_TICK_INDEX_OFFSET..ORCA_TICK_INDEX_OFFSET + 4].try_into()?,
        ),
        liquidity: u128::from_le_bytes(
            data[ORCA_LIQUIDITY_OFFSET..ORCA_LIQUIDITY_OFFSET + 16].try_into()?,
        ),
        tick_spacing: u16::from_le_bytes(
            data[ORCA_TICK_SPACING_OFFSET..ORCA_TICK_SPACING_OFFSET + 2].try_into()?,
        ),
        fee_rate: u16::from_le_bytes(
            data[ORCA_FEE_RATE_OFFSET..ORCA_FEE_RATE_OFFSET + 2].try_into()?,
        ),
    })
}

pub fn set_orca(state: &SharedState, data: &[u8]) -> Result<()> {
    let p = parse_whirlpool(data)?;
    let mut w = state.write().map_err(|_| anyhow!("state lock poisoned"))?;
    w.orca_sqrt_price = p.sqrt_price;
    w.orca_tick_index = p.tick_index;
    w.orca_liquidity = p.liquidity;
    w.orca_tick_spacing = p.tick_spacing;
    w.orca_fee_rate = p.fee_rate;
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
    state
        .read()
        .map(|g| *g)
        .map_err(|_| anyhow!("state lock poisoned"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_whirlpool(sqrt: u128, tick: i32) -> Vec<u8> {
        let mut d = vec![0u8; 653];
        d[ORCA_LIQUIDITY_OFFSET..ORCA_LIQUIDITY_OFFSET + 16]
            .copy_from_slice(&1_090_735_051_258_027u128.to_le_bytes());
        d[ORCA_TICK_SPACING_OFFSET..ORCA_TICK_SPACING_OFFSET + 2]
            .copy_from_slice(&4u16.to_le_bytes());
        d[ORCA_FEE_RATE_OFFSET..ORCA_FEE_RATE_OFFSET + 2].copy_from_slice(&400u16.to_le_bytes());
        d[ORCA_SQRT_PRICE_OFFSET..ORCA_SQRT_PRICE_OFFSET + 16].copy_from_slice(&sqrt.to_le_bytes());
        d[ORCA_TICK_INDEX_OFFSET..ORCA_TICK_INDEX_OFFSET + 4].copy_from_slice(&tick.to_le_bytes());
        d
    }

    #[test]
    fn parses_whirlpool_fields() {
        let p = parse_whirlpool(&fake_whirlpool(6_429_939_587_537_051_150, -21080)).unwrap();
        assert_eq!(p.sqrt_price, 6_429_939_587_537_051_150);
        assert_eq!(p.tick_index, -21080);
        assert_eq!(p.liquidity, 1_090_735_051_258_027);
        assert_eq!(p.tick_spacing, 4);
        assert_eq!(p.fee_rate, 400);
    }

    #[test]
    fn short_account_errors() {
        assert!(parse_whirlpool(&[0u8; 50]).is_err());
    }

    #[test]
    fn staleness() {
        let st = new_shared();
        set_ray_sol(&st, 1).unwrap();
        let mut s = snapshot(&st).unwrap();
        assert!(!s.is_stale(5));
        s.last_update -= 60;
        assert!(s.is_stale(5));
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
