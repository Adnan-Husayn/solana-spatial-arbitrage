//! Profit model: finds the trade size that maximizes SOL profit across both venues.
//!
//! Every cycle starts and ends in SOL, so profit is `sol_out - sol_in` minus fixed costs.

use crate::math::calculate_swap_out;
use crate::orca::{quote_sol_to_usdc, quote_usdc_to_sol};
use crate::pricing::Direction;
use crate::state::MarketState;

#[derive(Debug, Clone, Copy)]
pub struct StrategyConfig {
    pub min_trade_lamports: u64,
    pub max_trade_lamports: u64,
    /// Jito tip paid per bundle.
    pub tip_lamports: u64,
    /// Base signature fee plus priority fee for the transaction.
    pub tx_fee_lamports: u64,
    /// Opportunities with a lower net profit are ignored.
    pub min_net_profit_lamports: u64,
}

impl Default for StrategyConfig {
    fn default() -> Self {
        Self {
            min_trade_lamports: 10_000_000,       // 0.01 SOL
            max_trade_lamports: 50_000_000_000,   // 50 SOL
            tip_lamports: 10_000,
            tx_fee_lamports: 25_000,
            min_net_profit_lamports: 10_000,
        }
    }
}

impl StrategyConfig {
    pub fn fixed_cost(&self) -> u64 {
        self.tip_lamports + self.tx_fee_lamports
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Opportunity {
    pub direction: Direction,
    pub amount_in: u64,
    pub expected_out: u64,
    pub gross_profit: i64,
    pub cost: u64,
    pub net_profit: i64,
}

/// USDC received from the first leg of a SOL-in cycle.
pub fn first_leg_usdc(st: &MarketState, direction: Direction, sol_in: u64) -> Option<u64> {
    match direction {
        Direction::BuyRaydiumSellOrca => quote_sol_to_usdc(st, sol_in),
        Direction::BuyOrcaSellRaydium => calculate_swap_out(sol_in, st.ray_sol, st.ray_usdc).ok(),
    }
}

/// SOL received from the second leg given the USDC from the first.
pub fn second_leg_sol(st: &MarketState, direction: Direction, usdc: u64) -> Option<u64> {
    match direction {
        Direction::BuyRaydiumSellOrca => calculate_swap_out(usdc, st.ray_usdc, st.ray_sol).ok(),
        Direction::BuyOrcaSellRaydium => quote_usdc_to_sol(st, usdc),
    }
}

/// SOL out for a SOL-in round trip, or `None` if a leg can't be quoted.
/// BuyRaydiumSellOrca: sell SOL on Orca (richer), buy it back on Raydium.
/// BuyOrcaSellRaydium: sell SOL on Raydium (richer), buy it back on Orca.
pub fn round_trip(st: &MarketState, direction: Direction, sol_in: u64) -> Option<u64> {
    second_leg_sol(st, direction, first_leg_usdc(st, direction, sol_in)?)
}

fn gross(st: &MarketState, d: Direction, x: u64) -> Option<i128> {
    round_trip(st, d, x).map(|out| out as i128 - x as i128)
}

/// Best size for one direction. Profit is concave in size (price impact grows), but quotes
/// can fail past the Orca range, so scan a geometric grid first and refine around the best.
fn best_size(st: &MarketState, d: Direction, cfg: &StrategyConfig) -> Option<(u64, i128)> {
    const GRID: u32 = 48;
    let (min, max) = (cfg.min_trade_lamports.max(1), cfg.max_trade_lamports);
    if min > max {
        return None;
    }
    let ratio = (max as f64 / min as f64).powf(1.0 / GRID as f64);
    let points: Vec<u64> = (0..=GRID).map(|i| (min as f64 * ratio.powi(i as i32)).round() as u64).collect();

    let (idx, _) = points
        .iter()
        .enumerate()
        .filter_map(|(i, &x)| gross(st, d, x).map(|g| (i, g)))
        .max_by_key(|&(_, g)| g)?;

    let mut lo = points[idx.saturating_sub(1)];
    let mut hi = points[(idx + 1).min(points.len() - 1)];
    while hi - lo > 2 {
        let m1 = lo + (hi - lo) / 3;
        let m2 = hi - (hi - lo) / 3;
        let (g1, g2) = (gross(st, d, m1), gross(st, d, m2));
        match (g1, g2) {
            (Some(a), Some(b)) if a < b => lo = m1,
            (Some(_), Some(_)) => hi = m2,
            (Some(_), None) => hi = m2,
            (None, _) => lo = m1,
        }
    }
    (lo..=hi).filter_map(|x| gross(st, d, x).map(|g| (x, g))).max_by_key(|&(_, g)| g)
}

/// Returns the most profitable opportunity above the configured threshold, if any.
pub fn evaluate(st: &MarketState, cfg: &StrategyConfig) -> Option<Opportunity> {
    if !st.is_ready() {
        return None;
    }
    let cost = cfg.fixed_cost();
    [Direction::BuyRaydiumSellOrca, Direction::BuyOrcaSellRaydium]
        .into_iter()
        .filter_map(|d| {
            let (x, g) = best_size(st, d, cfg)?;
            let net = g - cost as i128;
            (net >= cfg.min_net_profit_lamports as i128).then(|| Opportunity {
                direction: d,
                amount_in: x,
                expected_out: (x as i128 + g) as u64,
                gross_profit: g as i64,
                cost,
                net_profit: net as i64,
            })
        })
        .max_by_key(|o| o.net_profit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orca::fixtures::snapshot;

    /// Move the Orca price by `pct` percent, keeping sqrt price and tick consistent.
    fn with_orca_shift(mut st: MarketState, pct: f64) -> MarketState {
        let sqrt = st.orca_sqrt_price as f64 * (1.0 + pct / 100.0).sqrt();
        st.orca_sqrt_price = sqrt as u128;
        let price = (sqrt / 18_446_744_073_709_551_616.0).powi(2);
        st.orca_tick_index = (price.ln() / 1.0001f64.ln()).floor() as i32;
        st
    }

    #[test]
    fn no_opportunity_when_spread_is_below_fees() {
        // Real snapshot: ~10 bps spread vs ~29 bps of pool fees.
        assert!(evaluate(&snapshot(), &StrategyConfig::default()).is_none());
    }

    #[test]
    fn orca_richer_means_sell_on_orca() {
        let st = with_orca_shift(snapshot(), 1.0);
        let o = evaluate(&st, &StrategyConfig::default()).expect("1% spread should be profitable");
        assert_eq!(o.direction, Direction::BuyRaydiumSellOrca);
        assert!(o.amount_in > 0 && o.net_profit > 0);
        assert_eq!(o.gross_profit - o.cost as i64, o.net_profit);
        assert_eq!(o.expected_out as i64 - o.amount_in as i64, o.gross_profit);
    }

    #[test]
    fn raydium_richer_means_sell_on_raydium() {
        let st = with_orca_shift(snapshot(), -1.0);
        let o = evaluate(&st, &StrategyConfig::default()).expect("1% spread should be profitable");
        assert_eq!(o.direction, Direction::BuyOrcaSellRaydium);
        assert!(o.net_profit > 0);
    }

    #[test]
    fn chosen_size_beats_the_coarse_grid() {
        let st = with_orca_shift(snapshot(), 1.0);
        let cfg = StrategyConfig::default();
        let o = evaluate(&st, &cfg).unwrap();
        for k in 0..20 {
            let x = cfg.min_trade_lamports as f64 * 1.6f64.powi(k);
            if x > cfg.max_trade_lamports as f64 {
                break;
            }
            if let Some(g) = gross(&st, o.direction, x as u64) {
                assert!(o.gross_profit as i128 >= g, "size {x} gave {g} > {}", o.gross_profit);
            }
        }
    }

    #[test]
    fn costs_can_erase_the_edge() {
        let st = with_orca_shift(snapshot(), 1.0);
        let cfg = StrategyConfig { tip_lamports: 100_000_000_000, ..StrategyConfig::default() };
        assert!(evaluate(&st, &cfg).is_none());
    }

    #[test]
    fn min_profit_threshold_is_respected() {
        let st = with_orca_shift(snapshot(), 1.0);
        let cfg = StrategyConfig { min_net_profit_lamports: u64::MAX / 4, ..StrategyConfig::default() };
        assert!(evaluate(&st, &cfg).is_none());
    }

    #[test]
    fn not_ready_state_yields_nothing() {
        assert!(evaluate(&MarketState::default(), &StrategyConfig::default()).is_none());
    }
}
