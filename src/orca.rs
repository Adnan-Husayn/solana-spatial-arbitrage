//! Single-range Whirlpool swap quotes.
//!
//! Between initialized ticks liquidity `L` is constant, so a swap moves the sqrt price along
//! a closed-form curve. We only trust the quote while the price stays inside the current
//! tick-spacing-aligned range; anything that would leave it is rejected, because liquidity
//! beyond that depends on tick arrays we don't read yet.
//!
//! Token A = SOL (input for A->B), token B = USDC. Amounts are raw units.

use crate::state::MarketState;

const Q64: f64 = 18_446_744_073_709_551_616.0;
const FEE_DENOM: f64 = 1_000_000.0;

fn sqrt_at_tick(tick: i32) -> f64 {
    1.0001f64.powf(tick as f64 / 2.0)
}

/// Sqrt-price bounds (as real numbers, not Q64) of the tick range containing the current price.
fn range(st: &MarketState) -> Option<(f64, f64)> {
    let spacing = st.orca_tick_spacing as i32;
    if spacing == 0 {
        return None;
    }
    let lower = st.orca_tick_index.div_euclid(spacing) * spacing;
    Some((sqrt_at_tick(lower), sqrt_at_tick(lower + spacing)))
}

fn usable(st: &MarketState) -> Option<(f64, f64)> {
    if st.orca_liquidity == 0 || st.orca_sqrt_price == 0 {
        return None;
    }
    Some((st.orca_sqrt_price as f64 / Q64, st.orca_liquidity as f64))
}

/// SOL in -> USDC out. `None` if the swap leaves the current range or state is unusable.
pub fn quote_sol_to_usdc(st: &MarketState, sol_in: u64) -> Option<u64> {
    let (s, l) = usable(st)?;
    let (lo, _) = range(st)?;
    let dx = sol_in as f64 * (1.0 - st.orca_fee_rate as f64 / FEE_DENOM);
    let s_new = l * s / (l + dx * s);
    if s_new < lo {
        return None;
    }
    Some((l * (s - s_new)).max(0.0).floor() as u64)
}

/// USDC in -> SOL out. `None` if the swap leaves the current range or state is unusable.
pub fn quote_usdc_to_sol(st: &MarketState, usdc_in: u64) -> Option<u64> {
    let (s, l) = usable(st)?;
    let (_, hi) = range(st)?;
    let dy = usdc_in as f64 * (1.0 - st.orca_fee_rate as f64 / FEE_DENOM);
    let s_new = s + dy / l;
    if s_new > hi {
        return None;
    }
    Some((l * (1.0 / s - 1.0 / s_new)).max(0.0).floor() as u64)
}

#[cfg(test)]
pub(crate) mod fixtures {
    use crate::state::MarketState;

    /// Mainnet snapshot, 2026-10-06.
    pub fn snapshot() -> MarketState {
        MarketState {
            ray_sol: 116_647_489_467_425,
            ray_usdc: 14_158_027_040_682,
            orca_sqrt_price: 6_425_431_916_669_708_712,
            orca_tick_index: -21094,
            orca_liquidity: 1_090_735_051_258_027,
            orca_tick_spacing: 4,
            orca_fee_rate: 400,
            last_update: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::snapshot;
    use super::*;
    use crate::pricing::orca_price;

    const SOL: u64 = 1_000_000_000;

    #[test]
    fn small_sell_matches_spot_minus_fee() {
        let st = snapshot();
        let out = quote_sol_to_usdc(&st, SOL / 100).unwrap() as f64 / 1e6;
        let expected = orca_price(st.orca_sqrt_price) * 0.01 * (1.0 - 0.0004);
        assert!((out - expected).abs() / expected < 1e-3, "{out} vs {expected}");
    }

    #[test]
    fn small_buy_matches_spot_minus_fee() {
        let st = snapshot();
        let out = quote_usdc_to_sol(&st, 1_000_000).unwrap() as f64 / 1e9; // 1 USDC
        let expected = 1.0 / orca_price(st.orca_sqrt_price) * (1.0 - 0.0004);
        assert!((out - expected).abs() / expected < 1e-3, "{out} vs {expected}");
    }

    #[test]
    fn round_trip_loses_about_two_fees() {
        let st = snapshot();
        let usdc = quote_sol_to_usdc(&st, SOL).unwrap();
        let back = quote_usdc_to_sol(&st, usdc).unwrap();
        let loss = 1.0 - back as f64 / SOL as f64;
        assert!(loss > 0.0007 && loss < 0.0012, "loss {loss}");
    }

    #[test]
    fn output_is_monotonic_with_price_impact() {
        let st = snapshot();
        let a = quote_sol_to_usdc(&st, SOL).unwrap() as f64;
        let b = quote_sol_to_usdc(&st, 100 * SOL).unwrap() as f64;
        assert!(b > a);
        assert!(b / 100.0 < a, "bigger trades must get a worse average price");
    }

    #[test]
    fn leaving_the_range_is_rejected() {
        let st = snapshot();
        assert!(quote_sol_to_usdc(&st, 1_000_000 * SOL).is_none());
        assert!(quote_usdc_to_sol(&st, u64::MAX).is_none());
    }

    #[test]
    fn empty_pool_is_rejected() {
        let mut st = snapshot();
        st.orca_liquidity = 0;
        assert!(quote_sol_to_usdc(&st, SOL).is_none());
    }
}
