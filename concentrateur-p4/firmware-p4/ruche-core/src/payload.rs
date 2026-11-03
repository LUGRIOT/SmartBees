//! Topics et payloads MQTT V27 (spec §2.6). Tout passe par serde_json :
//! aucune chaine venue d'un appareil n'est jamais concatenee a la main
//! (NFR-SE-03 ; la v3 faisait du snprintf("%s") sur des donnees reseau).
use crate::alarm::AlarmEvent;
use crate::audio::WindowSummary;
use crate::bridge::Bridge;
use crate::c6_proto::{Event, Ieee};
use crate::names;
use crate::zcl::{self, Ctx};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub fn topic(hive_id: &str, suffix: &str) -> String {
    format!("ruche/{hive_id}/{suffix}")
}

fn r(v: f64, decimals: i32) -> Value {
    let p = 10f64.powi(decimals);
    json!((v * p).round() / p)
}

#[derive(Debug, Clone, Copy)]
pub struct Env {
    pub temperature: f64,
    pub humidity: f64,
    pub pressure: f64,
}

pub struct DataInputs<'a> {
    pub hive_id: &'a str,
    /// Heure Unix si l'horloge a ete synchronisee (SNTP), sinon None
    pub timestamp: Option<u64>,
    pub uptime_ms: u64,
    pub fw: &'a str,
    pub env: Option<Env>,
    pub weight_kg: Option<f64>,
    pub audio: Option<&'a WindowSummary>,
    pub audio_state: &'a str,
    pub zigbee_online: bool,
    pub zigbee_devices: usize,
    pub zigbee_available: usize,
}

/// `ruche/<id>/data`. Memes noms, memes arrondis que build_payload() v3,
/// `satellites` en moins (FR-MQ-02).
pub fn data(d: &DataInputs) -> String {
    let env = d.env;
    let mut m = Map::new();
    m.insert("hive_id".into(), json!(d.hive_id));
    m.insert("timestamp".into(), json!(d.timestamp));
    if d.timestamp.is_none() {
        m.insert("uptime_ms".into(), json!(d.uptime_ms));
    }
    m.insert("fw".into(), json!(d.fw));
    m.insert(
        "temperature".into(),
        env.map_or(Value::Null, |e| r(e.temperature, 1)),
    );
    m.insert(
        "humidity".into(),
        env.map_or(Value::Null, |e| r(e.humidity, 0)),
    );
    m.insert(
        "pressure".into(),
        env.map_or(Value::Null, |e| r(e.pressure, 0)),
    );
    m.insert(
        "weight_kg".into(),
        d.weight_kg.map_or(Value::Null, |w| r(w, 2)),
    );
    // micro en panne : la fenetre a eu lieu, mais ses mesures sont fausses.
    // null plutot que 0, sinon le cloud tracerait une ruche muette.
    let measured = d.audio.filter(|a| !a.mic_fault);
    m.insert(
        "sound_hz".into(),
        measured.map_or(Value::Null, |a| r(a.f_dom as f64, 0)),
    );
    m.insert(
        "db_level".into(),
        measured.map_or(Value::Null, |a| r(a.db as f64, 1)),
    );
    m.insert(
        "audio".into(),
        json!({
            "bands": measured.map(|a| a.bands.map(|b| r(b as f64, 2))),
            "state": d.audio_state,
            "window_s": d.audio.map(|a| a.frames as f64 * crate::audio::FRAME as f64 / crate::audio::SAMPLE_RATE as f64).map(|s| s.round()),
        }),
    );
    m.insert(
        "zigbee".into(),
        json!({"online": d.zigbee_online, "devices": d.zigbee_devices, "available": d.zigbee_available}),
    );
    Value::Object(m).to_string()
}

/// `ruche/<id>/zigbee/<friendly_name>` : derniere valeur de chaque propriete.
pub fn device(bridge: &Bridge, ieee: Ieee) -> Option<String> {
    let st = bridge.states.get(&ieee)?;
    let mut m: Map<String, Value> = st
        .props
        .iter()
        .map(|(k, p)| (k.clone(), p.v.clone()))
        .collect();
    m.insert("linkquality".into(), json!(st.lqi));
    m.insert("last_seen".into(), json!(st.last_seen));
    Some(Value::Object(m).to_string())
}

/// `ruche/<id>/bridge/devices` (retained).
pub fn bridge_devices(bridge: &Bridge, names_map: &BTreeMap<Ieee, String>) -> String {
    let list: Vec<Value> = bridge
        .devices
        .values()
        .map(|d| {
            let ctx = Ctx {
                model: &d.model,
                zone_type: d.zone_type,
                analog_units: d.analog_units,
            };
            json!({
                "ieee": d.ieee,
                "friendly_name": names::effective(names_map, d.ieee),
                "manufacturer": d.mfr,
                "model": d.model,
                "power": d.power,
                "exposes": zcl::exposes(&d.clusters, &ctx),
            })
        })
        .collect();
    Value::Array(list).to_string()
}

/// `ruche/<id>/bridge/event`. `PortalRequest` n'est pas publie.
pub fn bridge_event(ev: &Event) -> Option<String> {
    let v = match ev {
        Event::Joined(i) => json!({"type": "device_joined", "ieee": i}),
        Event::Left(i) => json!({"type": "device_left", "ieee": i}),
        Event::Interview { ieee, ok: true } => json!({"type": "device_interviewed", "ieee": ieee}),
        Event::Interview { ieee, ok: false } => json!({"type": "interview_failed", "ieee": ieee}),
        Event::PortalRequest => return None,
    };
    Some(v.to_string())
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Errors {
    pub uart: u32,
    pub lte: u32,
    pub sd: u32,
}

/// `ruche/<id>/bridge/state` (retained ; le LWT est `bridge_offline()`).
#[derive(Debug, Clone, Serialize)]
pub struct BridgeState<'a> {
    pub fw_p4: &'a str,
    pub fw_c6: Option<&'a str>,
    pub uptime_s: u64,
    pub reset_reason: &'a str,
    pub err: Errors,
    pub queue: u32,
    pub zb_dropped: u32,
}

pub fn bridge_state(s: &BridgeState) -> String {
    let mut v = serde_json::to_value(s).expect("struct serialisable");
    v["state"] = json!("online");
    v.to_string()
}

pub fn bridge_offline() -> String {
    json!({"state": "offline"}).to_string()
}

/// `ruche/<id>/alarm` (QoS1). `rule` vaut null a la retombee.
pub fn alarm(
    ev: AlarmEvent,
    since: Option<u64>,
    w: &WindowSummary,
    windows: u32,
    recording: Option<&str>,
) -> String {
    json!({
        "type": "acoustic",
        "state": ev,
        "since": since,
        "rule": if ev == AlarmEvent::Cleared { None } else { w.breach },
        "metrics": {"db": r(w.db as f64, 1), "f_dom": r(w.f_dom as f64, 0), "ratio_b3_b2": r(w.ratio_b3_b2 as f64, 2)},
        "windows": windows,
        "recording": recording,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::Rule;
    use crate::c6_proto::{DeviceInfo, Power, Report};

    fn summary() -> WindowSummary {
        WindowSummary {
            frames: 117,
            db: 62.34,
            f_dom: 248.2,
            bands: [0.211, 0.52, 0.19, 0.079],
            ratio_b3_b2: 0.365,
            breach: Some(Rule::FreqMax),
            mic_fault: false,
        }
    }

    fn inputs<'a>(w: &'a WindowSummary) -> DataInputs<'a> {
        DataInputs {
            hive_id: "cca-ruche-0001",
            timestamp: Some(1792540800),
            uptime_ms: 5,
            fw: "27.01.0",
            env: Some(Env {
                temperature: 24.14,
                humidity: 57.6,
                pressure: 951.2,
            }),
            weight_kg: Some(41.853),
            audio: Some(w),
            audio_state: "normal",
            zigbee_online: true,
            zigbee_devices: 3,
            zigbee_available: 3,
        }
    }

    #[test]
    fn data_keeps_v3_fields() {
        let w = summary();
        let v: Value = serde_json::from_str(&data(&inputs(&w))).unwrap();
        for k in [
            "hive_id",
            "timestamp",
            "temperature",
            "humidity",
            "pressure",
            "weight_kg",
            "sound_hz",
            "db_level",
        ] {
            assert!(v.get(k).is_some(), "champ v3 manquant : {k}");
        }
        assert!(v.get("satellites").is_none());
        assert!(v.get("uptime_ms").is_none());
        assert_eq!(v["temperature"], json!(24.1));
        assert_eq!(v["humidity"], json!(58.0));
        assert_eq!(v["weight_kg"], json!(41.85));
        assert_eq!(v["sound_hz"], json!(248.0));
        assert_eq!(v["db_level"], json!(62.3));
        assert_eq!(v["audio"]["window_s"], json!(30.0));
        assert_eq!(v["zigbee"]["devices"], json!(3));
    }

    #[test]
    fn data_mic_fault_is_null_not_zero() {
        let w = WindowSummary {
            frames: 117,
            db: 0.0,
            f_dom: 0.0,
            bands: [0.0; 4],
            ratio_b3_b2: 0.0,
            breach: None,
            mic_fault: true,
        };
        let d = DataInputs {
            audio_state: "mic_fault",
            ..inputs(&w)
        };
        let v: Value = serde_json::from_str(&data(&d)).unwrap();
        assert_eq!(v["sound_hz"], Value::Null);
        assert_eq!(v["db_level"], Value::Null);
        assert_eq!(v["audio"]["bands"], Value::Null);
        assert_eq!(v["audio"]["state"], json!("mic_fault"));
        assert_eq!(v["audio"]["window_s"], json!(30.0));
    }

    #[test]
    fn data_nulls_when_missing() {
        let w = summary();
        let d = DataInputs {
            timestamp: None,
            env: None,
            weight_kg: None,
            audio: None,
            ..inputs(&w)
        };
        let v: Value = serde_json::from_str(&data(&d)).unwrap();
        assert_eq!(v["timestamp"], Value::Null);
        assert_eq!(v["uptime_ms"], json!(5));
        assert_eq!(v["temperature"], Value::Null);
        assert_eq!(v["weight_kg"], Value::Null);
        assert_eq!(v["db_level"], Value::Null);
    }

    #[test]
    fn hostile_device_strings_are_escaped() {
        let mut b = Bridge::default();
        b.set_devices(vec![DeviceInfo {
            ieee: Ieee(1),
            nwk: 1,
            mfr: "a\"b".into(),
            model: "<script>x</script>".into(),
            power: Power::Mains,
            eps: vec![1],
            clusters: vec![0x0402],
            lqi: 1,
            last_seen_s: 0,
            zone_type: None,
            analog_units: None,
        }]);
        let s = bridge_devices(&b, &BTreeMap::new());
        let v: Value = serde_json::from_str(&s).expect("JSON valide malgre les guillemets");
        assert_eq!(v[0]["manufacturer"], json!("a\"b"));
        assert_eq!(v[0]["model"], json!("<script>x</script>"));
        assert_eq!(v[0]["friendly_name"], json!("0x0000000000000001"));
        assert_eq!(v[0]["exposes"], json!(["temperature"]));
    }

    #[test]
    fn device_payload() {
        let mut b = Bridge::default();
        b.apply(
            &Report {
                seq: 1,
                ieee: Ieee(1),
                ep: 1,
                cl: 0x0402,
                at: 0,
                ty: 41,
                v: 2150.0,
                lqi: 176,
                age_s: 0,
            },
            100,
        );
        let v: Value = serde_json::from_str(&device(&b, Ieee(1)).unwrap()).unwrap();
        assert_eq!(
            v,
            json!({"temperature": 21.5, "linkquality": 176, "last_seen": 100})
        );
        assert!(device(&b, Ieee(2)).is_none());
    }

    #[test]
    fn alarm_and_bridge_payloads() {
        let w = summary();
        let v: Value = serde_json::from_str(&alarm(
            AlarmEvent::Raised,
            Some(10),
            &w,
            2,
            Some("audio/x.wav"),
        ))
        .unwrap();
        assert_eq!(v["state"], json!("raised"));
        assert_eq!(v["rule"], json!("freq_max"));
        assert_eq!(v["metrics"]["ratio_b3_b2"], json!(0.37));
        let c: Value =
            serde_json::from_str(&alarm(AlarmEvent::Cleared, None, &w, 0, None)).unwrap();
        assert_eq!(c["rule"], Value::Null);
        let st = BridgeState {
            fw_p4: "27.01.0",
            fw_c6: None,
            uptime_s: 1,
            reset_reason: "deepsleep",
            err: Errors::default(),
            queue: 0,
            zb_dropped: 0,
        };
        let s: Value = serde_json::from_str(&bridge_state(&st)).unwrap();
        assert_eq!(s["state"], json!("online"));
        assert_eq!(s["err"]["lte"], json!(0));
        assert_eq!(bridge_event(&Event::PortalRequest), None);
        assert_eq!(
            topic("cca-ruche-0001", "alarm"),
            "ruche/cca-ruche-0001/alarm"
        );
    }
}
