//! Reglages persistants (NVS `ruche27`), bornes et valeurs par defaut :
//! spec §2.9. Les bornes sont verifiees ici et nulle part ailleurs ; le
//! portail et la NVS passent tous deux par `validate`.
use crate::c6_proto::Ieee;
use serde::{Deserialize, Serialize};

/// Memes valeurs que `ALLOWED_INTERVALS` de power_mgr.c en v3.
pub const ALLOWED_INTERVALS_S: [u32; 10] =
    [60, 300, 900, 1800, 3600, 7200, 14400, 21600, 43200, 86400];

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecMode {
    Off,
    OnAlarm,
    Always,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub interval_s: u32,
    pub audio_window_s: u16,
    pub spl_offset_db: f32,
    pub lvl_min_db: f32,
    pub lvl_max_db: f32,
    pub f_min_hz: f32,
    pub f_max_hz: f32,
    pub ratio_b3_b2_max: f32,
    pub breach_ratio: f32,
    pub confirm_windows: u8,
    pub clear_windows: u8,
    pub alarm_cooldown_min: u16,
    pub rec_mode: RecMode,
    pub rec_quota_mb: u32,
    pub portal_idle_min: u16,
    pub weight_ieee: Option<Ieee>,
    pub weight_max_age_s: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            interval_s: 300,
            audio_window_s: 30,
            // +120 : valeur de la v3 (sensors.c), a etalonner (T-SYS-04)
            spl_offset_db: 120.0,
            lvl_min_db: 40.0,
            lvl_max_db: 85.0,
            // colonie normale ~200-300 Hz, piping / pre-essaimage ~400-500 Hz
            f_min_hz: 150.0,
            f_max_hz: 400.0,
            ratio_b3_b2_max: 1.5,
            breach_ratio: 0.6,
            confirm_windows: 2,
            clear_windows: 2,
            alarm_cooldown_min: 60,
            rec_mode: RecMode::OnAlarm,
            rec_quota_mb: 1024,
            portal_idle_min: 10,
            weight_ieee: None,
            weight_max_age_s: 7200,
        }
    }
}

/// Nom du champ fautif, renvoye tel quel au portail.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigError(pub &'static str);

fn within<T: PartialOrd>(v: T, lo: T, hi: T, field: &'static str) -> Result<(), ConfigError> {
    if v >= lo && v <= hi {
        Ok(())
    } else {
        Err(ConfigError(field))
    }
}

/// 90 % de la SD (spec 2.9) ; sans SD, pas de borne haute verifiable.
fn quota_max_mb(sd_capacity_mb: Option<u32>) -> u32 {
    sd_capacity_mb.map_or(u32::MAX, |c| (c as u64 * 9 / 10) as u32)
}

impl Settings {
    /// Reglages d'usine ajustes a la SD montee : seul le quota audio
    /// depend de la carte.
    pub fn for_sd(sd_capacity_mb: Option<u32>) -> Self {
        let d = Settings::default();
        Settings {
            rec_quota_mb: d.rec_quota_mb.min(quota_max_mb(sd_capacity_mb)),
            ..d
        }
    }

    /// `sd_capacity_mb` : None si la SD est absente, la borne haute du
    /// quota n'est alors pas verifiable et n'est pas appliquee.
    pub fn validate(&self, sd_capacity_mb: Option<u32>) -> Result<(), ConfigError> {
        if !ALLOWED_INTERVALS_S.contains(&self.interval_s) {
            return Err(ConfigError("interval_s"));
        }
        within(self.audio_window_s, 5, 120, "audio_window_s")?;
        within(self.spl_offset_db, 80.0, 140.0, "spl_offset_db")?;
        within(self.lvl_min_db, 20.0, 120.0, "lvl_min_db")?;
        within(self.lvl_max_db, 20.0, 120.0, "lvl_max_db")?;
        if self.lvl_min_db >= self.lvl_max_db {
            return Err(ConfigError("lvl_min_db"));
        }
        within(self.f_min_hz, 50.0, 2000.0, "f_min_hz")?;
        within(self.f_max_hz, 50.0, 2000.0, "f_max_hz")?;
        if self.f_min_hz >= self.f_max_hz {
            return Err(ConfigError("f_min_hz"));
        }
        within(self.ratio_b3_b2_max, 0.1, 10.0, "ratio_b3_b2_max")?;
        within(self.breach_ratio, 0.1, 1.0, "breach_ratio")?;
        within(self.confirm_windows, 1, 10, "confirm_windows")?;
        within(self.clear_windows, 1, 10, "clear_windows")?;
        within(self.alarm_cooldown_min, 5, 1440, "alarm_cooldown_min")?;
        within(
            self.rec_quota_mb,
            64,
            quota_max_mb(sd_capacity_mb),
            "rec_quota_mb",
        )?;
        within(self.portal_idle_min, 2, 60, "portal_idle_min")?;
        within(self.weight_max_age_s, 600, 86400, "weight_max_age_s")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        assert_eq!(Settings::default().validate(Some(8192)), Ok(()));
        assert_eq!(Settings::default().validate(None), Ok(()));
    }

    #[test]
    fn each_bound_rejected() {
        let d = Settings::default;
        let cases: Vec<(Settings, &str)> = vec![
            (
                Settings {
                    interval_s: 120,
                    ..d()
                },
                "interval_s",
            ),
            (
                Settings {
                    audio_window_s: 4,
                    ..d()
                },
                "audio_window_s",
            ),
            (
                Settings {
                    audio_window_s: 121,
                    ..d()
                },
                "audio_window_s",
            ),
            (
                Settings {
                    spl_offset_db: 79.0,
                    ..d()
                },
                "spl_offset_db",
            ),
            (
                Settings {
                    lvl_min_db: 85.0,
                    ..d()
                },
                "lvl_min_db",
            ),
            (
                Settings {
                    f_min_hz: 400.0,
                    ..d()
                },
                "f_min_hz",
            ),
            (
                Settings {
                    f_max_hz: 2001.0,
                    ..d()
                },
                "f_max_hz",
            ),
            (
                Settings {
                    ratio_b3_b2_max: 0.0,
                    ..d()
                },
                "ratio_b3_b2_max",
            ),
            (
                Settings {
                    breach_ratio: 1.1,
                    ..d()
                },
                "breach_ratio",
            ),
            (
                Settings {
                    confirm_windows: 0,
                    ..d()
                },
                "confirm_windows",
            ),
            (
                Settings {
                    clear_windows: 11,
                    ..d()
                },
                "clear_windows",
            ),
            (
                Settings {
                    alarm_cooldown_min: 4,
                    ..d()
                },
                "alarm_cooldown_min",
            ),
            (
                Settings {
                    rec_quota_mb: 63,
                    ..d()
                },
                "rec_quota_mb",
            ),
            (
                Settings {
                    portal_idle_min: 61,
                    ..d()
                },
                "portal_idle_min",
            ),
            (
                Settings {
                    weight_max_age_s: 599,
                    ..d()
                },
                "weight_max_age_s",
            ),
        ];
        for (s, field) in cases {
            assert_eq!(s.validate(Some(8192)), Err(ConfigError(field)));
        }
        // quota > 90 % d'une SD de 1 Go
        let q = Settings {
            rec_quota_mb: 1000,
            ..d()
        };
        assert_eq!(q.validate(Some(1024)), Err(ConfigError("rec_quota_mb")));
        assert_eq!(q.validate(None), Ok(()));
    }

    #[test]
    fn defaults_fit_small_sd() {
        // 1024 Mo par defaut > 90 % d'une SD de 1 Go : sans ajustement, les
        // reglages d'usine seraient refuses et rien ne s'enregistrerait
        let s = Settings::for_sd(Some(1000));
        assert_eq!(s.rec_quota_mb, 900);
        assert_eq!(s.validate(Some(1000)), Ok(()));
        assert_eq!(Settings::for_sd(Some(8192)), Settings::default());
        assert_eq!(Settings::for_sd(None), Settings::default());
    }

    #[test]
    fn json_partial_and_roundtrip() {
        let s: Settings =
            serde_json::from_str(r#"{"rec_mode":"always","weight_ieee":"0x00124b0012345678"}"#)
                .unwrap();
        assert_eq!(s.rec_mode, RecMode::Always);
        assert_eq!(s.weight_ieee, Some(Ieee(0x00124b0012345678)));
        assert_eq!(s.interval_s, 300);
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
