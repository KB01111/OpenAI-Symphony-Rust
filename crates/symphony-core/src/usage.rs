//! Token accounting for provider threads that outlive a single agent session.
//!
//! Codex reports `thread/tokenUsage/updated` with the **absolute** usage of the whole thread
//! (`tokenUsage.total`), and a resumed thread starts with the usage of every earlier turn. Verified
//! against `codex app-server` 0.163.0: `thread/resume` replays the persisted total as a
//! `thread/tokenUsage/updated` notification (carrying the id of the thread's *previous* turn)
//! before any new turn runs. Measured naively from zero, that replay would be charged again to the
//! task and to the runtime totals on every retry.
//!
//! The ledger separates the quantities involved:
//!
//! * **historical thread usage** – what the thread had consumed before this session (`baseline`),
//! * **previously credited usage** – the part of that history Symphony already accounted for
//!   (the persisted baseline handed to [`ThreadUsageLedger::resumed`]),
//! * **current-session usage** – `thread total - baseline`, what this session adds,
//!
//! and callers derive current-turn usage as the growth of the session usage across a turn.
//! When the provider exposes too little to tell history from new usage, the ledger says so through
//! [`ThreadUsageLedger::is_estimated`] instead of reporting false precision.

use crate::domain::TokenUsage;

/// Whether a usage report belongs to the turn the session is running or is a replay of earlier
/// state (sent while resuming a thread, or carrying the id of an earlier turn).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportOrigin {
    Replay,
    Live,
}

#[derive(Debug, Clone)]
pub struct ThreadUsageLedger {
    /// Usage the thread had already consumed before this session started.
    baseline: TokenUsage,
    /// Resumed without a persisted baseline: the first replayed report establishes it.
    baseline_pending: bool,
    /// Latest absolute thread total (never decreases).
    thread_total: TokenUsage,
    estimated: bool,
}

impl ThreadUsageLedger {
    /// A thread started by this session: everything it reports is new usage.
    pub fn fresh() -> Self {
        Self {
            baseline: TokenUsage::default(),
            baseline_pending: false,
            thread_total: TokenUsage::default(),
            estimated: false,
        }
    }

    /// A resumed thread. `accounted` is the absolute thread usage earlier attempts already
    /// accounted for (`None` when it was never recorded, e.g. for tasks stored by an older
    /// version).
    pub fn resumed(accounted: Option<TokenUsage>) -> Self {
        match accounted {
            Some(baseline) => Self {
                baseline,
                baseline_pending: false,
                thread_total: baseline,
                estimated: false,
            },
            None => Self {
                baseline: TokenUsage::default(),
                baseline_pending: true,
                thread_total: TokenUsage::default(),
                estimated: false,
            },
        }
    }

    /// Record an absolute thread total reported by the provider.
    pub fn observe(&mut self, reported_total: TokenUsage, origin: ReportOrigin) {
        if !covers(&reported_total, &self.baseline) {
            // A thread's total never shrinks, so the recorded baseline cannot belong to this
            // thread's history: count the reported total as new usage, but flag it.
            self.baseline = TokenUsage::default();
            self.thread_total = TokenUsage::default();
            self.baseline_pending = false;
            self.estimated = true;
        } else if self.baseline_pending {
            self.baseline_pending = false;
            match origin {
                ReportOrigin::Replay => self.baseline = reported_total,
                // No replay was seen, so the first live total may still contain history.
                ReportOrigin::Live => self.estimated = true,
            }
        }
        self.thread_total = self.thread_total.max(&reported_total);
    }

    /// Usage this session added to the thread (cumulative over all its turns).
    pub fn session_usage(&self) -> TokenUsage {
        self.thread_total.since(&self.baseline)
    }

    /// Latest absolute thread total, to persist as the baseline for the next resume.
    pub fn thread_total(&self) -> TokenUsage {
        self.thread_total
    }

    /// `true` when [`Self::session_usage`] may include usage that predates this session.
    pub fn is_estimated(&self) -> bool {
        self.estimated
    }
}

fn covers(total: &TokenUsage, floor: &TokenUsage) -> bool {
    total.input_tokens >= floor.input_tokens
        && total.output_tokens >= floor.output_tokens
        && total.total_tokens >= floor.total_tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u64, output: u64) -> TokenUsage {
        TokenUsage {
            input_tokens: input,
            output_tokens: output,
            total_tokens: input + output,
        }
    }

    #[test]
    fn fresh_thread_counts_everything_and_ignores_repeated_reports() {
        let mut ledger = ThreadUsageLedger::fresh();
        ledger.observe(usage(100, 20), ReportOrigin::Live);
        ledger.observe(usage(100, 20), ReportOrigin::Live);
        ledger.observe(usage(300, 50), ReportOrigin::Live);
        assert_eq!(ledger.session_usage(), usage(300, 50));
        assert_eq!(ledger.thread_total(), usage(300, 50));
        assert!(!ledger.is_estimated());
    }

    #[test]
    fn reports_never_move_the_total_backwards() {
        let mut ledger = ThreadUsageLedger::fresh();
        ledger.observe(usage(300, 50), ReportOrigin::Live);
        ledger.observe(usage(120, 10), ReportOrigin::Live);
        assert_eq!(ledger.session_usage(), usage(300, 50));
    }

    #[test]
    fn replay_of_the_accounted_total_is_not_charged_again() {
        let mut ledger = ThreadUsageLedger::resumed(Some(usage(1000, 500)));
        ledger.observe(usage(1000, 500), ReportOrigin::Replay);
        assert_eq!(ledger.session_usage(), TokenUsage::default());
        ledger.observe(usage(1400, 650), ReportOrigin::Live);
        assert_eq!(ledger.session_usage(), usage(400, 150));
        assert_eq!(ledger.thread_total(), usage(1400, 650));
        assert!(!ledger.is_estimated());
    }

    #[test]
    fn usage_lost_to_a_crashed_turn_is_credited_exactly_once() {
        // The previous attempt crashed after the thread reached 1300 but before the 1000 total
        // was the last one persisted: the replay carries the 300 that were never accounted for.
        let mut ledger = ThreadUsageLedger::resumed(Some(usage(900, 100)));
        ledger.observe(usage(1100, 200), ReportOrigin::Replay);
        assert_eq!(ledger.session_usage(), usage(200, 100));
        ledger.observe(usage(1100, 200), ReportOrigin::Live);
        assert_eq!(ledger.session_usage(), usage(200, 100));
    }

    #[test]
    fn unknown_baseline_takes_the_replayed_total() {
        let mut ledger = ThreadUsageLedger::resumed(None);
        ledger.observe(usage(1000, 500), ReportOrigin::Replay);
        assert_eq!(ledger.session_usage(), TokenUsage::default());
        ledger.observe(usage(1250, 560), ReportOrigin::Live);
        assert_eq!(ledger.session_usage(), usage(250, 60));
        assert!(!ledger.is_estimated());
    }

    #[test]
    fn unknown_baseline_without_replay_is_flagged_as_estimated() {
        let mut ledger = ThreadUsageLedger::resumed(None);
        ledger.observe(usage(1250, 560), ReportOrigin::Live);
        // Possibly includes earlier turns: reported, but not as an exact figure.
        assert_eq!(ledger.session_usage(), usage(1250, 560));
        assert!(ledger.is_estimated());
    }

    #[test]
    fn a_baseline_larger_than_the_thread_total_is_discarded_and_flagged() {
        // E.g. the thread was replaced by a new one that kept its id.
        let mut ledger = ThreadUsageLedger::resumed(Some(usage(5000, 900)));
        ledger.observe(usage(200, 40), ReportOrigin::Live);
        assert_eq!(ledger.session_usage(), usage(200, 40));
        assert!(ledger.is_estimated());
    }

    #[test]
    fn resumed_session_without_any_report_adds_nothing() {
        let ledger = ThreadUsageLedger::resumed(Some(usage(1000, 500)));
        assert_eq!(ledger.session_usage(), TokenUsage::default());
        assert_eq!(ledger.thread_total(), usage(1000, 500));
    }
}
