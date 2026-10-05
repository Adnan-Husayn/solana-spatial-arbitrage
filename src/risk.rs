//! Guard rails for live trading. Everything here is pure and testable; the caller supplies
//! the clock, balances and kill-switch state.
//!
//! Live sending is off unless `LIVE=true`. Even then, every trade must pass `RiskState::check`.

use crate::strategy::Opportunity;
use std::fmt;
use std::path::PathBuf;

const SECS_PER_DAY: u64 = 86_400;

#[derive(Debug, Clone)]
pub struct RiskConfig {
    pub live: bool,
    /// Hard cap on input size per trade, applied on top of the strategy's own maximum.
    pub max_trade_lamports: u64,
    /// Trading halts for the rest of the UTC day once realized losses reach this.
    pub max_daily_loss_lamports: u64,
    /// Native SOL that must stay in the wallet (covers fees and tips).
    pub min_balance_lamports: u64,
    /// Trading halts while this file exists.
    pub kill_file: PathBuf,
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            live: false,
            max_trade_lamports: 1_000_000_000,    // 1 SOL
            max_daily_loss_lamports: 100_000_000, // 0.1 SOL
            min_balance_lamports: 50_000_000,     // 0.05 SOL
            kill_file: PathBuf::from("KILL"),
        }
    }
}

impl RiskConfig {
    /// Reads `LIVE`, `MAX_LIVE_TRADE_SOL`, `MAX_DAILY_LOSS_SOL`, `MIN_BALANCE_SOL`, `KILL_FILE`.
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let sol = |key: &str| -> anyhow::Result<Option<u64>> {
            get(key)
                .map(|v| {
                    let x: f64 = v
                        .trim()
                        .parse()
                        .map_err(|_| anyhow::anyhow!("invalid value for {key}: {v:?}"))?;
                    if !x.is_finite() || x <= 0.0 {
                        anyhow::bail!("{key} must be a positive number, got {v:?}");
                    }
                    Ok((x * 1e9).round() as u64)
                })
                .transpose()
        };

        let mut cfg = Self::default();
        if let Some(v) = get("LIVE") {
            cfg.live = match v.trim().to_ascii_lowercase().as_str() {
                "true" => true,
                "false" | "" => false,
                other => anyhow::bail!("LIVE must be 'true' or 'false', got {other:?}"),
            };
        }
        if let Some(v) = sol("MAX_LIVE_TRADE_SOL")? {
            cfg.max_trade_lamports = v;
        }
        if let Some(v) = sol("MAX_DAILY_LOSS_SOL")? {
            cfg.max_daily_loss_lamports = v;
        }
        if let Some(v) = sol("MIN_BALANCE_SOL")? {
            cfg.min_balance_lamports = v;
        }
        if let Some(v) = get("KILL_FILE") {
            cfg.kill_file = PathBuf::from(v);
        }
        Ok(cfg)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reject {
    KillSwitch,
    DailyLossReached { loss: u64, limit: u64 },
    LowBalance { have: u64, need: u64 },
    TradeTooLarge { size: u64, max: u64 },
}

impl fmt::Display for Reject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reject::KillSwitch => write!(f, "kill switch is active"),
            Reject::DailyLossReached { loss, limit } => {
                write!(f, "daily loss {loss} lamports reached the limit of {limit}")
            }
            Reject::LowBalance { have, need } => {
                write!(f, "balance {have} lamports is below the {need} required")
            }
            Reject::TradeTooLarge { size, max } => write!(f, "trade size {size} exceeds cap {max}"),
        }
    }
}

#[derive(Debug, Default)]
pub struct RiskState {
    day: u64,
    /// Realized profit and loss for the current UTC day, in lamports.
    pnl: i64,
}

impl RiskState {
    pub fn new() -> Self {
        Self::default()
    }

    fn roll(&mut self, now_unix_secs: u64) {
        let day = now_unix_secs / SECS_PER_DAY;
        if day != self.day {
            self.day = day;
            self.pnl = 0;
        }
    }

    pub fn pnl(&mut self, now_unix_secs: u64) -> i64 {
        self.roll(now_unix_secs);
        self.pnl
    }

    /// Records the realized change in total wallet value (native SOL plus WSOL) from a trade.
    pub fn record_pnl(&mut self, now_unix_secs: u64, delta_lamports: i64) {
        self.roll(now_unix_secs);
        self.pnl = self.pnl.saturating_add(delta_lamports);
    }

    /// Decides whether a trade may be sent.
    pub fn check(
        &mut self,
        cfg: &RiskConfig,
        now_unix_secs: u64,
        kill_switch_active: bool,
        native_balance: u64,
        opp: &Opportunity,
    ) -> Result<(), Reject> {
        self.roll(now_unix_secs);
        if kill_switch_active {
            return Err(Reject::KillSwitch);
        }
        if self.pnl < 0 && self.pnl.unsigned_abs() >= cfg.max_daily_loss_lamports {
            return Err(Reject::DailyLossReached {
                loss: self.pnl.unsigned_abs(),
                limit: cfg.max_daily_loss_lamports,
            });
        }
        if opp.amount_in > cfg.max_trade_lamports {
            return Err(Reject::TradeTooLarge {
                size: opp.amount_in,
                max: cfg.max_trade_lamports,
            });
        }
        let need = cfg.min_balance_lamports.saturating_add(opp.cost);
        if native_balance < need {
            return Err(Reject::LowBalance {
                have: native_balance,
                need,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pricing::Direction;

    const DAY: u64 = SECS_PER_DAY;

    fn opp(amount_in: u64) -> Opportunity {
        Opportunity {
            direction: Direction::BuyRaydiumSellOrca,
            amount_in,
            expected_out: amount_in + 1_000_000,
            gross_profit: 1_000_000,
            cost: 35_000,
            net_profit: 965_000,
        }
    }

    fn lookup(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pairs
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn live_is_off_by_default_and_limits_are_conservative() {
        let cfg = RiskConfig::from_lookup(lookup(&[])).unwrap();
        assert!(!cfg.live);
        assert_eq!(cfg.max_trade_lamports, 1_000_000_000);
        assert_eq!(cfg.max_daily_loss_lamports, 100_000_000);
        assert_eq!(cfg.min_balance_lamports, 50_000_000);
    }

    #[test]
    fn config_overrides_and_validation() {
        let cfg = RiskConfig::from_lookup(lookup(&[
            ("LIVE", "true"),
            ("MAX_LIVE_TRADE_SOL", "0.25"),
            ("KILL_FILE", "/tmp/stop"),
        ]))
        .unwrap();
        assert!(cfg.live);
        assert_eq!(cfg.max_trade_lamports, 250_000_000);
        assert_eq!(cfg.kill_file, PathBuf::from("/tmp/stop"));
        assert!(RiskConfig::from_lookup(lookup(&[("LIVE", "yes")])).is_err());
        assert!(RiskConfig::from_lookup(lookup(&[("MAX_DAILY_LOSS_SOL", "-1")])).is_err());
        assert!(RiskConfig::from_lookup(lookup(&[("MIN_BALANCE_SOL", "abc")])).is_err());
    }

    #[test]
    fn allows_a_normal_trade() {
        let mut r = RiskState::new();
        assert_eq!(
            r.check(
                &RiskConfig::default(),
                DAY,
                false,
                1_000_000_000,
                &opp(500_000_000)
            ),
            Ok(())
        );
    }

    #[test]
    fn kill_switch_blocks_everything() {
        let mut r = RiskState::new();
        assert_eq!(
            r.check(&RiskConfig::default(), DAY, true, 10_000_000_000, &opp(1)),
            Err(Reject::KillSwitch)
        );
    }

    #[test]
    fn oversized_trades_are_rejected() {
        let mut r = RiskState::new();
        let res = r.check(
            &RiskConfig::default(),
            DAY,
            false,
            10_000_000_000,
            &opp(1_000_000_001),
        );
        assert!(matches!(res, Err(Reject::TradeTooLarge { .. })));
    }

    #[test]
    fn low_balance_counts_the_trade_cost() {
        let mut r = RiskState::new();
        let cfg = RiskConfig::default();
        // exactly min balance + cost is fine, one lamport less is not
        assert!(r.check(&cfg, DAY, false, 50_035_000, &opp(1_000)).is_ok());
        assert!(matches!(
            r.check(&cfg, DAY, false, 50_034_999, &opp(1_000)),
            Err(Reject::LowBalance { .. })
        ));
    }

    #[test]
    fn daily_loss_cap_halts_until_the_next_day() {
        let mut r = RiskState::new();
        let cfg = RiskConfig::default();
        r.record_pnl(DAY, -60_000_000);
        assert!(
            r.check(&cfg, DAY, false, 10_000_000_000, &opp(1_000))
                .is_ok()
        );
        r.record_pnl(DAY + 10, -40_000_000);
        assert!(matches!(
            r.check(&cfg, DAY + 20, false, 10_000_000_000, &opp(1_000)),
            Err(Reject::DailyLossReached { .. })
        ));
        // next UTC day resets
        assert!(
            r.check(&cfg, 2 * DAY, false, 10_000_000_000, &opp(1_000))
                .is_ok()
        );
        assert_eq!(r.pnl(2 * DAY), 0);
    }

    #[test]
    fn profits_offset_losses() {
        let mut r = RiskState::new();
        r.record_pnl(DAY, -90_000_000);
        r.record_pnl(DAY, 50_000_000);
        assert!(
            r.check(
                &RiskConfig::default(),
                DAY,
                false,
                10_000_000_000,
                &opp(1_000)
            )
            .is_ok()
        );
    }
}
