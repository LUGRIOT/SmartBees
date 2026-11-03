//! Converters ZCL -> proprietes JSON, a la maniere des `fromZigbee` de
//! zigbee-herdsman-converters (spec §2.2, §2.5.3). Uniquement les
//! clusters standard ; les noms de proprietes sont ceux de Zigbee2MQTT
//! pour que la comparaison T-INT-04 soit directe.
use serde_json::{json, Value};

/// Modele annonce par le noeud de pesee NAU7802 (spec §2.3.2).
pub const WEIGHT_MODEL: &str = "CCA-RUCHE-WEIGHT";
/// EngineeringUnits BACnet « kilograms ».
pub const UNITS_KG: u16 = 39;

/// Ce que l'interview a appris de l'appareil et qui change la conversion.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ctx<'a> {
    pub model: &'a str,
    pub zone_type: Option<u16>,
    pub analog_units: Option<u16>,
}

fn round(v: f64, decimals: i32) -> Value {
    let p = 10f64.powi(decimals);
    json!((v * p).round() / p)
}

/// Convertit un attribut en zero, une ou plusieurs proprietes.
/// Les valeurs « invalides » definies par le ZCL ne produisent rien.
pub fn convert(cluster: u16, attr: u16, raw: f64, ctx: &Ctx) -> Vec<(&'static str, Value)> {
    let one = |k, v| vec![(k, v)];
    match (cluster, attr) {
        (0x0402, 0x0000) if raw != -32768.0 => one("temperature", round(raw / 100.0, 2)),
        (0x0405, 0x0000) if raw != 65535.0 => one("humidity", round(raw / 100.0, 2)),
        (0x0403, 0x0000) if raw != -32768.0 => one("pressure", round(raw, 1)),
        (0x0400, 0x0000) if raw != 65535.0 => {
            let lux = if raw <= 0.0 {
                0.0
            } else {
                10f64.powf((raw - 1.0) / 10000.0)
            };
            one("illuminance", round(lux, 0))
        }
        (0x0001, 0x0021) if raw != 255.0 => one("battery", round(raw / 2.0, 0)),
        (0x0001, 0x0020) if raw != 255.0 => one("voltage", json!(raw * 100.0)),
        (0x0406, 0x0000) => one("occupancy", json!(raw as u32 & 1 == 1)),
        (0x0006, 0x0000) => one(
            "state",
            json!(if raw as u32 & 1 == 1 { "ON" } else { "OFF" }),
        ),
        (0x040D, 0x0000) if raw.is_finite() => one("co2", round(raw * 1e6, 0)),
        (0x042A, 0x0000) if raw.is_finite() => one("pm25", round(raw, 0)),
        (0x000C, 0x0055) if raw.is_finite() => {
            if ctx.model == WEIGHT_MODEL || ctx.analog_units == Some(UNITS_KG) {
                one("weight", round(raw, 2))
            } else {
                one("analog_value", json!(raw))
            }
        }
        // IAS Zone : ZoneStatus, bit 0 = alarme 1, bit 3 = batterie faible
        (0x0500, 0x0002) => {
            let st = raw as u32;
            let alarm = st & 1 == 1;
            let main = match ctx.zone_type {
                Some(0x0015) => ("contact", json!(!alarm)),
                Some(0x002A) => ("water_leak", json!(alarm)),
                Some(0x000D) => ("occupancy", json!(alarm)),
                _ => ("alarm", json!(alarm)),
            };
            vec![main, ("battery_low", json!(st & 0b1000 != 0))]
        }
        _ => vec![],
    }
}

/// Proprietes exposees, deduites des clusters vus a l'interview
/// (equivalent simplifie des `exposes` de Zigbee2MQTT).
pub fn exposes(clusters: &[u16], ctx: &Ctx) -> Vec<&'static str> {
    let mut out = vec![];
    for c in clusters {
        let p: &[&'static str] = match c {
            0x0402 => &["temperature"],
            0x0405 => &["humidity"],
            0x0403 => &["pressure"],
            0x0400 => &["illuminance"],
            0x0001 => &["battery", "voltage"],
            0x0406 => &["occupancy"],
            0x0006 => &["state"],
            0x040D => &["co2"],
            0x042A => &["pm25"],
            0x000C => {
                if ctx.model == WEIGHT_MODEL || ctx.analog_units == Some(UNITS_KG) {
                    &["weight"]
                } else {
                    &["analog_value"]
                }
            }
            _ => &[],
        };
        out.extend_from_slice(p);
        if *c == 0x0500 {
            out.extend(
                convert(0x0500, 0x0002, 0.0, ctx)
                    .into_iter()
                    .map(|(k, _)| k),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(cl: u16, at: u16, raw: f64) -> Vec<(&'static str, Value)> {
        convert(cl, at, raw, &Ctx::default())
    }

    #[test]
    fn standard_clusters() {
        assert_eq!(c(0x0402, 0, 2150.0), vec![("temperature", json!(21.5))]);
        assert_eq!(c(0x0402, 0, -1234.0), vec![("temperature", json!(-12.34))]);
        assert_eq!(c(0x0405, 0, 6420.0), vec![("humidity", json!(64.2))]);
        assert_eq!(c(0x0403, 0, 951.0), vec![("pressure", json!(951.0))]);
        assert_eq!(c(0x0400, 0, 1.0), vec![("illuminance", json!(1.0))]);
        assert_eq!(c(0x0400, 0, 30001.0), vec![("illuminance", json!(1000.0))]);
        assert_eq!(c(0x0400, 0, 0.0), vec![("illuminance", json!(0.0))]);
        assert_eq!(c(0x0001, 0x21, 174.0), vec![("battery", json!(87.0))]);
        assert_eq!(c(0x0001, 0x20, 30.0), vec![("voltage", json!(3000.0))]);
        assert_eq!(c(0x0406, 0, 1.0), vec![("occupancy", json!(true))]);
        assert_eq!(c(0x0006, 0, 0.0), vec![("state", json!("OFF"))]);
        assert_eq!(c(0x040D, 0, 0.000412), vec![("co2", json!(412.0))]);
        assert_eq!(c(0x042A, 0, 12.4), vec![("pm25", json!(12.0))]);
    }

    #[test]
    fn zcl_invalid_values_ignored() {
        assert!(c(0x0402, 0, -32768.0).is_empty());
        assert!(c(0x0405, 0, 65535.0).is_empty());
        assert!(c(0x0403, 0, -32768.0).is_empty());
        assert!(c(0x0400, 0, 65535.0).is_empty());
        assert!(c(0x0001, 0x21, 255.0).is_empty());
        assert!(c(0x0001, 0x20, 255.0).is_empty());
        assert!(c(0x040D, 0, f64::NAN).is_empty());
        assert!(c(0x9999, 0, 1.0).is_empty());
        assert!(c(0x0402, 0x0001, 1.0).is_empty());
    }

    #[test]
    fn analog_input_weight_or_generic() {
        let w = Ctx {
            model: WEIGHT_MODEL,
            ..Default::default()
        };
        assert_eq!(
            convert(0x000C, 0x55, 41.853, &w),
            vec![("weight", json!(41.85))]
        );
        let kg = Ctx {
            analog_units: Some(UNITS_KG),
            ..Default::default()
        };
        assert_eq!(
            convert(0x000C, 0x55, 3.0, &kg),
            vec![("weight", json!(3.0))]
        );
        assert_eq!(c(0x000C, 0x55, 3.5), vec![("analog_value", json!(3.5))]);
    }

    #[test]
    fn ias_zone_by_type() {
        let contact = Ctx {
            zone_type: Some(0x0015),
            ..Default::default()
        };
        assert_eq!(
            convert(0x0500, 2, 0b0001 as f64, &contact),
            vec![("contact", json!(false)), ("battery_low", json!(false))]
        );
        let leak = Ctx {
            zone_type: Some(0x002A),
            ..Default::default()
        };
        assert_eq!(
            convert(0x0500, 2, 0b1001 as f64, &leak),
            vec![("water_leak", json!(true)), ("battery_low", json!(true))]
        );
        assert_eq!(c(0x0500, 2, 1.0)[0], ("alarm", json!(true)));
    }

    #[test]
    fn exposes_from_clusters() {
        let ctx = Ctx {
            zone_type: Some(0x0015),
            ..Default::default()
        };
        assert_eq!(
            exposes(&[0x0000, 0x0402, 0x0405, 0x0001, 0x0500], &ctx),
            vec![
                "temperature",
                "humidity",
                "battery",
                "voltage",
                "contact",
                "battery_low"
            ]
        );
        let w = Ctx {
            model: WEIGHT_MODEL,
            ..Default::default()
        };
        assert_eq!(exposes(&[0x000C], &w), vec!["weight"]);
    }
}
