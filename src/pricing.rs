//! Pure pricing math: no I/O, fully unit-testable.
//! All prices are USDC per 1 SOL.

use crate::config::{SOL_DECIMALS, USDC_DECIMALS};

const Q64: f64 = 18_446_744_073_709_551_616.0; // 2^64

fn decimals_adjust() -> f64 {
    10f64.powi(SOL_DECIMALS as i32 - USDC_DECIMALS as i32)
}

/// Orca price from Q64.64 sqrt price. Token A = SOL, token B = USDC.
pub fn orca_price(sqrt_price_x64: u128) -> f64 {
    let s = sqrt_price_x64 as f64 / Q64;
    s * s * decimals_adjust()
}

/// Raydium spot price from vault reserves (raw units).
pub fn raydium_price(sol_reserve: u64, usdc_reserve: u64) -> Option<f64> {
    if sol_reserve == 0 {
        return None;
    }
    let sol = sol_reserve as f64 / 10f64.powi(SOL_DECIMALS as i32);
    let usdc = usdc_reserve as f64 / 10f64.powi(USDC_DECIMALS as i32);
    Some(usdc / sol)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Buy SOL on Raydium, sell on Orca (Orca is richer).
    BuyRaydiumSellOrca,
    /// Buy SOL on Orca, sell on Raydium (Raydium is richer).
    BuyOrcaSellRaydium,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spread {
    pub direction: Direction,
    /// Gross spread in basis points, always >= 0, before any fees.
    pub bps: f64,
    pub ray_price: f64,
    pub orca_price: f64,
}

pub fn spread(ray_price: f64, orca_price: f64) -> Option<Spread> {
    if ray_price <= 0.0 || orca_price <= 0.0 || !ray_price.is_finite() || !orca_price.is_finite() {
        return None;
    }
    let (direction, low, high) = if orca_price >= ray_price {
        (Direction::BuyRaydiumSellOrca, ray_price, orca_price)
    } else {
        (Direction::BuyOrcaSellRaydium, orca_price, ray_price)
    };
    Some(Spread {
        direction,
        bps: (high - low) / low * 10_000.0,
        ray_price,
        orca_price,
    })
}

/// Convenience wrapper over raw on-chain state.
pub fn spread_from_state(ray_sol: u64, ray_usdc: u64, orca_sqrt: u128) -> Option<Spread> {
    let ray = raydium_price(ray_sol, ray_usdc)?;
    spread(ray, orca_price(orca_sqrt))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mainnet snapshot, 2026-10-06.
    const ORCA_SQRT: u128 = 6_429_939_587_537_051_150;
    const RAY_SOL: u64 = 116_647_489_467_425;
    const RAY_USDC: u64 = 14_158_027_040_682;

    #[test]
    fn orca_price_matches_snapshot() {
        let p = orca_price(ORCA_SQRT);
        assert!((p - 121.4995).abs() < 0.01, "got {p}");
    }

    #[test]
    fn orca_price_agrees_with_tick() {
        // tick -21080 => 1.0001^tick * 1e3
        let from_tick = 1.0001f64.powi(-21080) * 1e3;
        assert!((orca_price(ORCA_SQRT) - from_tick).abs() / from_tick < 1e-3);
    }

    #[test]
    fn raydium_price_matches_snapshot() {
        let p = raydium_price(RAY_SOL, RAY_USDC).unwrap();
        assert!((p - 121.37).abs() < 0.05, "got {p}");
    }

    #[test]
    fn price_of_one_is_decimal_adjusted() {
        // sqrt_price = 2^64 => raw price 1 => 1e3 USDC/SOL after decimals
        assert!((orca_price(1u128 << 64) - 1000.0).abs() < 1e-9);
    }

    #[test]
    fn raydium_zero_reserve_is_none() {
        assert!(raydium_price(0, 100).is_none());
    }

    #[test]
    fn spread_direction_and_bps() {
        let s = spread(100.0, 101.0).unwrap();
        assert_eq!(s.direction, Direction::BuyRaydiumSellOrca);
        assert!((s.bps - 100.0).abs() < 1e-9);

        let s = spread(101.0, 100.0).unwrap();
        assert_eq!(s.direction, Direction::BuyOrcaSellRaydium);
        assert!((s.bps - 100.0).abs() < 1e-9);
    }

    #[test]
    fn spread_rejects_bad_input() {
        assert!(spread(0.0, 100.0).is_none());
        assert!(spread(f64::NAN, 100.0).is_none());
    }

    #[test]
    fn snapshot_spread_is_small() {
        let s = spread_from_state(RAY_SOL, RAY_USDC, ORCA_SQRT).unwrap();
        assert!(s.bps < 20.0, "got {}", s.bps);
    }
}
