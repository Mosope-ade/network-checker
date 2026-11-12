use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Tracker {
    pub failures: u32,
    pub successes: u32,
    pub down: bool,
    pub first_failure: Option<i64>,
    pub first_success: Option<i64>,
}

#[derive(Debug, PartialEq)]
pub enum Transition {
    Down(i64),
    Recovered(i64),
}

impl Tracker {
    pub fn record(
        &mut self,
        ok: bool,
        now: i64,
        failure: u32,
        recovery: u32,
    ) -> Option<Transition> {
        if ok {
            self.failures = 0;
            self.first_failure = None;
            if !self.down {
                return None;
            }
            self.first_success.get_or_insert(now);
            self.successes = self.successes.saturating_add(1);
            if self.successes >= recovery {
                self.down = false;
                self.successes = 0;
                return Some(Transition::Recovered(self.first_success.take().unwrap()));
            }
        } else {
            self.successes = 0;
            self.first_success = None;
            self.first_failure.get_or_insert(now);
            self.failures = self.failures.saturating_add(1);
            if !self.down && self.failures >= failure {
                self.down = true;
                return Some(Transition::Down(self.first_failure.unwrap()));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outage_uses_first_failed_probe_and_recovery_uses_first_stable_success() {
        let mut t = Tracker::default();
        assert_eq!(t.record(false, 1000, 2, 2), None);
        assert_eq!(t.record(false, 2000, 2, 2), Some(Transition::Down(1000)));
        assert_eq!(t.record(false, 3000, 2, 2), None);
        assert_eq!(t.record(true, 4000, 2, 2), None);
        assert_eq!(
            t.record(true, 5000, 2, 2),
            Some(Transition::Recovered(4000))
        );
        assert!(!t.down);
    }
    #[test]
    fn transient_failure_does_not_open_incident_and_flapping_resets_recovery() {
        let mut t = Tracker::default();
        t.record(false, 1, 2, 2);
        t.record(true, 2, 2, 2);
        assert_eq!(t.record(false, 3, 2, 2), None);
        assert_eq!(t.record(false, 4, 2, 2), Some(Transition::Down(3)));
        t.record(true, 5, 2, 2);
        t.record(false, 6, 2, 2);
        assert_eq!(t.record(true, 7, 2, 2), None);
        assert_eq!(t.record(true, 8, 2, 2), Some(Transition::Recovered(7)));
    }
}
