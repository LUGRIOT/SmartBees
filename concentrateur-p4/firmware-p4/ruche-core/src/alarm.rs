//! Machine d'etats de l'alarme acoustique (FR-AL-01..04) :
//! Normal -> Suspect(n) -> Alarm -> Recovering(n) -> Normal.
//! Une transition par fenetre d'ecoute. L'etat est serialise par le
//! firmware (RTC RAM + NVS) car le P4 dort entre deux fenetres.
use crate::config::Settings;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Phase {
    Normal,
    Suspect(u8),
    Alarm,
    Recovering(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlarmEvent {
    Raised,
    Active,
    Cleared,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlarmState {
    pub phase: Phase,
    /// Instant de la levee de l'alarme en cours
    pub since: Option<u64>,
    last_pub: u64,
    /// Fenetres hors consigne consecutives
    pub streak: u32,
}

impl Default for AlarmState {
    fn default() -> Self {
        AlarmState {
            phase: Phase::Normal,
            since: None,
            last_pub: 0,
            streak: 0,
        }
    }
}

impl AlarmState {
    /// Fait avancer la machine d'une fenetre. `now` : horloge RTC en s.
    /// Une fenetre `mic_fault` n'apporte aucune information : etat inchange.
    pub fn step(
        &mut self,
        breach: bool,
        mic_fault: bool,
        now: u64,
        s: &Settings,
    ) -> Option<AlarmEvent> {
        if mic_fault {
            return None;
        }
        self.streak = if breach { self.streak + 1 } else { 0 };
        let confirm = s.confirm_windows;
        let clear = s.clear_windows;
        let (next, ev) = match (self.phase, breach) {
            (Phase::Normal, false) => (Phase::Normal, None),
            (Phase::Normal, true) | (Phase::Suspect(_), true) => {
                let n = match self.phase {
                    Phase::Suspect(n) => n + 1,
                    _ => 1,
                };
                if n >= confirm {
                    self.since = Some(now);
                    self.last_pub = now;
                    (Phase::Alarm, Some(AlarmEvent::Raised))
                } else {
                    (Phase::Suspect(n), None)
                }
            }
            (Phase::Suspect(_), false) => (Phase::Normal, None),
            (Phase::Alarm, true) | (Phase::Recovering(_), true) => {
                if now.saturating_sub(self.last_pub) >= s.alarm_cooldown_min as u64 * 60 {
                    self.last_pub = now;
                    (Phase::Alarm, Some(AlarmEvent::Active))
                } else {
                    (Phase::Alarm, None)
                }
            }
            (Phase::Alarm, false) | (Phase::Recovering(_), false) => {
                let n = match self.phase {
                    Phase::Recovering(n) => n + 1,
                    _ => 1,
                };
                if n >= clear {
                    self.since = None;
                    (Phase::Normal, Some(AlarmEvent::Cleared))
                } else {
                    (Phase::Recovering(n), None)
                }
            }
        };
        self.phase = next;
        ev
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(seq: &[bool], s: &Settings) -> Vec<Option<AlarmEvent>> {
        let mut a = AlarmState::default();
        seq.iter()
            .enumerate()
            .map(|(i, &b)| a.step(b, false, i as u64 * 300, s))
            .collect()
    }

    #[test]
    fn confirm_and_clear() {
        let s = Settings::default(); // confirm 2, clear 2
        use AlarmEvent::*;
        assert_eq!(
            run(
                &[true, false, true, true, true, false, true, false, false],
                &s
            ),
            vec![
                None,
                None,
                None,
                Some(Raised),
                None,
                None,
                None,
                None,
                Some(Cleared)
            ]
        );
    }

    #[test]
    fn confirm_one_raises_immediately() {
        let s = Settings {
            confirm_windows: 1,
            clear_windows: 1,
            ..Settings::default()
        };
        assert_eq!(
            run(&[true, false], &s),
            vec![Some(AlarmEvent::Raised), Some(AlarmEvent::Cleared)]
        );
    }

    #[test]
    fn cooldown_three_hours() {
        // FR-AL-04 : 3 h d'alarme continue, cycles de 5 min, cooldown 60 min
        let s = Settings::default();
        let evs = run(&[true; 37], &s);
        let raised = evs
            .iter()
            .filter(|e| **e == Some(AlarmEvent::Raised))
            .count();
        let active = evs
            .iter()
            .filter(|e| **e == Some(AlarmEvent::Active))
            .count();
        assert_eq!((raised, active), (1, 2));
        assert!(active <= 3);
    }

    #[test]
    fn mic_fault_freezes_state() {
        let s = Settings::default();
        let mut a = AlarmState::default();
        a.step(true, false, 0, &s);
        let before = a.clone();
        assert_eq!(a.step(false, true, 300, &s), None);
        assert_eq!(a, before);
        assert_eq!(a.step(true, false, 600, &s), Some(AlarmEvent::Raised));
        assert_eq!(a.since, Some(600));
        assert_eq!(a.streak, 2);
    }

    #[test]
    fn serde_roundtrip() {
        let s = Settings::default();
        let mut a = AlarmState::default();
        a.step(true, false, 0, &s);
        a.step(true, false, 300, &s);
        let back: AlarmState = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(back, a);
    }
}
