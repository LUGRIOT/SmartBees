//! Le « zigbee2mqtt » du P4 : table des appareils (venue du C6) et
//! derniere valeur de chaque propriete. Serialisable : le firmware la
//! conserve a travers le deep sleep, sinon une pesee envoyee toutes les
//! heures serait absente de 11 cycles sur 12.
use crate::c6_proto::{DeviceInfo, Ieee, Report};
use crate::zcl::{self, Ctx, WEIGHT_MODEL};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stamped {
    pub v: Value,
    /// Horloge RTC du P4, en secondes
    pub ts: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeviceState {
    pub props: BTreeMap<String, Stamped>,
    pub lqi: u8,
    pub last_seen: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Bridge {
    pub devices: BTreeMap<Ieee, DeviceInfo>,
    pub states: BTreeMap<Ieee, DeviceState>,
    updated: BTreeSet<Ieee>,
}

impl Bridge {
    /// Remplace la table par celle du C6. Les etats des appareils exclus
    /// sont oublies ; ceux des appareils restants sont conserves.
    pub fn set_devices(&mut self, list: Vec<DeviceInfo>) {
        self.devices = list.into_iter().map(|d| (d.ieee, d)).collect();
        let devices = &self.devices;
        self.states.retain(|k, _| devices.contains_key(k));
        self.updated.retain(|k| devices.contains_key(k));
    }

    /// Applique un report. Un appareil absent de la table (appaire
    /// pendant le sommeil du P4, table pas encore relue) est converti
    /// avec les regles generiques plutot que perdu.
    pub fn apply(&mut self, r: &Report, now: u64) {
        let info = self.devices.get(&r.ieee);
        let ctx = Ctx {
            model: info.map_or("", |d| d.model.as_str()),
            zone_type: info.and_then(|d| d.zone_type),
            analog_units: info.and_then(|d| d.analog_units),
        };
        let ts = now.saturating_sub(r.age_s as u64);
        let props = zcl::convert(r.cl, r.at, r.v, &ctx);
        let st = self.states.entry(r.ieee).or_default();
        // reports relus dans l'ordre des seq : le dernier gagne
        st.lqi = r.lqi;
        st.last_seen = st.last_seen.max(ts);
        for (k, v) in props {
            st.props.insert(k.to_string(), Stamped { v, ts });
        }
        self.updated.insert(r.ieee);
    }

    /// Appareils modifies depuis le dernier appel (une publication MQTT
    /// par appareil et par cycle).
    pub fn take_updated(&mut self) -> Vec<Ieee> {
        std::mem::take(&mut self.updated).into_iter().collect()
    }

    /// Poids de LA ruche pour `data.weight_kg` (spec §2.3.2).
    pub fn weight_kg(&self, chosen: Option<Ieee>, now: u64, max_age_s: u32) -> Option<f64> {
        let ieee = match chosen {
            Some(i) => i,
            None => {
                let mut nodes = self.devices.values().filter(|d| d.model == WEIGHT_MODEL);
                match (nodes.next(), nodes.next()) {
                    (Some(d), None) => d.ieee,
                    _ => return None, // aucun, ou plusieurs : a choisir au portail
                }
            }
        };
        let w = self.states.get(&ieee)?.props.get("weight")?;
        if now.saturating_sub(w.ts) > max_age_s as u64 {
            return None;
        }
        w.v.as_f64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::c6_proto::Power;
    use serde_json::json;

    fn dev(ieee: u64, model: &str, clusters: Vec<u16>) -> DeviceInfo {
        DeviceInfo {
            ieee: Ieee(ieee),
            nwk: 1,
            mfr: "CCA".into(),
            model: model.into(),
            power: Power::Battery,
            eps: vec![1],
            clusters,
            lqi: 100,
            last_seen_s: 0,
            zone_type: None,
            analog_units: None,
        }
    }

    fn rep(ieee: u64, cl: u16, at: u16, v: f64, age_s: u32) -> Report {
        Report {
            seq: 1,
            ieee: Ieee(ieee),
            ep: 1,
            cl,
            at,
            ty: 0,
            v,
            lqi: 150,
            age_s,
        }
    }

    #[test]
    fn apply_keeps_latest_and_marks_updated() {
        let mut b = Bridge::default();
        b.set_devices(vec![dev(1, "lumi.weather", vec![0x0402])]);
        b.apply(&rep(1, 0x0402, 0, 2000.0, 200), 1000);
        b.apply(&rep(1, 0x0402, 0, 2150.0, 100), 1000);
        let st = &b.states[&Ieee(1)];
        assert_eq!(
            st.props["temperature"],
            Stamped {
                v: json!(21.5),
                ts: 900
            }
        );
        assert_eq!(st.last_seen, 900);
        assert_eq!(b.take_updated(), vec![Ieee(1)]);
        assert!(b.take_updated().is_empty());
    }

    #[test]
    fn unknown_device_still_converted() {
        let mut b = Bridge::default();
        b.apply(&rep(7, 0x0405, 0, 5000.0, 0), 10);
        assert_eq!(b.states[&Ieee(7)].props["humidity"].v, json!(50.0));
        assert_eq!(b.take_updated(), vec![Ieee(7)]);
    }

    #[test]
    fn removed_device_state_dropped() {
        let mut b = Bridge::default();
        b.set_devices(vec![dev(1, "a", vec![]), dev(2, "b", vec![])]);
        b.apply(&rep(2, 0x0402, 0, 1.0, 0), 10);
        b.set_devices(vec![dev(1, "a", vec![])]);
        assert!(!b.states.contains_key(&Ieee(2)));
        assert!(b.take_updated().is_empty());
    }

    #[test]
    fn weight_selection_and_staleness() {
        let mut b = Bridge::default();
        assert_eq!(b.weight_kg(None, 0, 7200), None);
        b.set_devices(vec![
            dev(1, WEIGHT_MODEL, vec![0x000C]),
            dev(2, "lumi.weather", vec![]),
        ]);
        b.apply(&rep(1, 0x000C, 0x55, 41.85, 0), 1000);
        assert_eq!(b.weight_kg(None, 1000, 7200), Some(41.85));
        assert_eq!(b.weight_kg(None, 1000 + 7200, 7200), Some(41.85));
        assert_eq!(b.weight_kg(None, 1000 + 7201, 7200), None);
        // deux noeuds de pesee : rien sans choix explicite
        let mut devs = vec![dev(1, WEIGHT_MODEL, vec![]), dev(3, WEIGHT_MODEL, vec![])];
        b.set_devices(devs.clone());
        assert_eq!(b.weight_kg(None, 1000, 7200), None);
        assert_eq!(b.weight_kg(Some(Ieee(1)), 1000, 7200), Some(41.85));
        assert_eq!(b.weight_kg(Some(Ieee(3)), 1000, 7200), None);
        devs.pop();
        b.set_devices(devs);
        // un report de batterie ne rafraichit pas l'age du poids
        b.apply(&rep(1, 0x0001, 0x21, 180.0, 0), 9000);
        assert_eq!(b.weight_kg(None, 9000, 7200), None);
    }

    #[test]
    fn survives_serde_roundtrip() {
        let mut b = Bridge::default();
        b.set_devices(vec![dev(1, WEIGHT_MODEL, vec![0x000C])]);
        b.apply(&rep(1, 0x000C, 0x55, 41.85, 0), 1000);
        let back: Bridge = serde_json::from_str(&serde_json::to_string(&b).unwrap()).unwrap();
        assert_eq!(back, b);
    }
}
