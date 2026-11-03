//! Protocole UART P4 <-> C6 v2 (spec §2.4) : une ligne JSON par message,
//! 512 octets maximum, `\n` final. Le P4 est maitre ; le C6 ne parle
//! spontanement que pour des evenements `{"ev":...}`.
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Map, Value};
use std::fmt;
use std::str::FromStr;

/// Trame complete, `\n` compris (NFR-SE-06).
pub const MAX_LINE: usize = 512;
/// Longueur maximale des chaines venant d'un appareil (fabricant, modele).
pub const MAX_STR: usize = 32;
pub const MAX_DEVICES: u8 = 32;

#[derive(Debug, Clone, PartialEq)]
pub enum ProtoError {
    TooLong,
    Json,
    Schema(&'static str),
    WrongId,
    Range(&'static str),
}

/// Adresse IEEE 64 bits, ecrite `0x` + 16 chiffres hexa sur le fil.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ieee(pub u64);

impl fmt::Display for Ieee {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "0x{:016x}", self.0)
    }
}

impl FromStr for Ieee {
    type Err = ProtoError;
    fn from_str(s: &str) -> Result<Self, ProtoError> {
        parse_hex(s, 16).map(Ieee).ok_or(ProtoError::Schema("ieee"))
    }
}

impl Serialize for Ieee {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Ieee {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(|_| de::Error::custom("ieee"))
    }
}

/// `0x` suivi de 1 a `max_digits` chiffres hexa.
fn parse_hex(s: &str, max_digits: usize) -> Option<u64> {
    let h = s.strip_prefix("0x")?;
    if h.is_empty() || h.len() > max_digits {
        return None;
    }
    u64::from_str_radix(h, 16).ok()
}

fn de_hex16<'de, D: Deserializer<'de>>(d: D) -> Result<u16, D::Error> {
    let s = String::deserialize(d)?;
    parse_hex(&s, 4)
        .map(|v| v as u16)
        .ok_or_else(|| de::Error::custom("hex16"))
}

fn de_hex64<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    let s = String::deserialize(d)?;
    parse_hex(&s, 16).ok_or_else(|| de::Error::custom("hex64"))
}

fn ser_hex16<S: Serializer>(v: &u16, s: S) -> Result<S::Ok, S::Error> {
    s.collect_str(&format_args!("0x{v:04x}"))
}

// ── Requetes P4 -> C6 ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Status,
    Devices,
    Reports { after: u32, max: u16 },
    PermitJoin { s: u8 },
    Remove { ieee: Ieee },
    ResetNetwork { ch: u8, pan: u16 },
    SetBlePasskey { pk: String },
}

/// Serialise une requete en ligne JSON terminee par `\n`.
/// Les bornes sont verifiees ICI, avant d'envoyer quoi que ce soit au C6.
pub fn encode_request(id: u16, req: &Request) -> Result<String, ProtoError> {
    let v = match req {
        Request::Status => json!({"id": id, "cmd": "status"}),
        Request::Devices => json!({"id": id, "cmd": "devices"}),
        Request::Reports { after, max } => {
            if *max == 0 || *max > 64 {
                return Err(ProtoError::Range("max"));
            }
            json!({"id": id, "cmd": "reports", "after": after, "max": max})
        }
        // 255 = ouverture permanente en Zigbee : interdit (FR-ZB-02)
        Request::PermitJoin { s } => {
            if !(1..=254).contains(s) {
                return Err(ProtoError::Range("s"));
            }
            json!({"id": id, "cmd": "permit_join", "s": s})
        }
        Request::Remove { ieee } => json!({"id": id, "cmd": "remove", "ieee": ieee}),
        Request::ResetNetwork { ch, pan } => {
            if !(11..=26).contains(ch) {
                return Err(ProtoError::Range("ch"));
            }
            // 0x0000 et 0xFFFF sont reserves par la norme
            if *pan == 0 || *pan == 0xFFFF {
                return Err(ProtoError::Range("pan"));
            }
            json!({"id": id, "cmd": "reset_network", "ch": ch,
                   "pan": format!("0x{pan:04x}"), "confirm": "RESET"})
        }
        Request::SetBlePasskey { pk } => {
            if pk.len() != 6 || !pk.bytes().all(|b| b.is_ascii_digit()) {
                return Err(ProtoError::Range("pk"));
            }
            json!({"id": id, "cmd": "set_ble_passkey", "pk": pk})
        }
    };
    Ok(format!("{v}\n"))
}

// ── Reponses et evenements C6 -> P4 ───────────────────────────────

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Status {
    pub fw: String,
    pub ch: u8,
    #[serde(deserialize_with = "de_hex16")]
    pub pan: u16,
    #[serde(deserialize_with = "de_hex64")]
    pub epan: u64,
    pub devices: u8,
    pub permit_join: u8,
    pub seq: u32,
    pub dropped: u32,
    pub uptime_s: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Power {
    Battery,
    Mains,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub ieee: Ieee,
    #[serde(deserialize_with = "de_hex16", serialize_with = "ser_hex16")]
    pub nwk: u16,
    pub mfr: String,
    pub model: String,
    pub power: Power,
    pub eps: Vec<u8>,
    pub clusters: Vec<u16>,
    pub lqi: u8,
    pub last_seen_s: u32,
    /// Attribut IAS Zone 0x0001, lu a l'interview si le cluster 0x0500 existe
    #[serde(default)]
    pub zone_type: Option<u16>,
    /// Attribut Analog Input 0x0075 (EngineeringUnits), lu a l'interview
    #[serde(default)]
    pub analog_units: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Report {
    pub seq: u32,
    pub ieee: Ieee,
    pub ep: u8,
    pub cl: u16,
    pub at: u16,
    pub ty: u8,
    pub v: f64,
    pub lqi: u8,
    pub age_s: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Joined(Ieee),
    Left(Ieee),
    Interview { ieee: Ieee, ok: bool },
    PortalRequest,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    Status(Status),
    Device(DeviceInfo),
    Report(Report),
    End { end: u32, more: bool },
    Ok,
    Err(String),
    Event(Event),
}

fn typed<T: de::DeserializeOwned>(
    m: Map<String, Value>,
    what: &'static str,
) -> Result<T, ProtoError> {
    serde_json::from_value(Value::Object(m)).map_err(|_| ProtoError::Schema(what))
}

fn ieee_field(m: &Map<String, Value>) -> Result<Ieee, ProtoError> {
    m.get("ieee")
        .and_then(Value::as_str)
        .ok_or(ProtoError::Schema("ieee"))?
        .parse()
}

/// Analyse une ligne recue (sans son `\n`). `expected_id` est l'id de
/// la requete en cours : une reponse portant un autre id est rejetee,
/// les evenements (sans id) sont toujours acceptes.
pub fn parse_line(expected_id: u16, line: &[u8]) -> Result<Line, ProtoError> {
    if line.len() >= MAX_LINE {
        return Err(ProtoError::TooLong);
    }
    let m = match serde_json::from_slice::<Value>(line) {
        Ok(Value::Object(m)) => m,
        _ => return Err(ProtoError::Json),
    };

    if let Some(ev) = m.get("ev").and_then(Value::as_str) {
        let e = match ev {
            "joined" => Event::Joined(ieee_field(&m)?),
            "left" => Event::Left(ieee_field(&m)?),
            "interview" => Event::Interview {
                ieee: ieee_field(&m)?,
                ok: m
                    .get("ok")
                    .and_then(Value::as_bool)
                    .ok_or(ProtoError::Schema("ok"))?,
            },
            "portal_request" => Event::PortalRequest,
            _ => return Err(ProtoError::Schema("ev")),
        };
        return Ok(Line::Event(e));
    }

    if m.get("id").and_then(Value::as_u64) != Some(expected_id as u64) {
        return Err(ProtoError::WrongId);
    }

    if let Some(e) = m.get("err") {
        return Ok(Line::Err(e.as_str().unwrap_or("?").to_string()));
    }
    if m.contains_key("ok") {
        return Ok(Line::Ok);
    }
    if let Some(end) = m.get("end") {
        let end = end
            .as_u64()
            .filter(|v| *v <= u32::MAX as u64)
            .ok_or(ProtoError::Schema("end"))?;
        let more = m.get("more").and_then(Value::as_bool).unwrap_or(false);
        return Ok(Line::End {
            end: end as u32,
            more,
        });
    }
    if m.contains_key("seq") && m.contains_key("ieee") {
        return Ok(Line::Report(typed(m, "report")?));
    }
    if m.contains_key("ieee") {
        let d: DeviceInfo = typed(m, "device")?;
        if d.mfr.len() > MAX_STR || d.model.len() > MAX_STR {
            return Err(ProtoError::Range("str"));
        }
        return Ok(Line::Device(d));
    }
    if m.contains_key("fw") {
        let s: Status = typed(m, "status")?;
        if !(11..=26).contains(&s.ch) {
            return Err(ProtoError::Range("ch"));
        }
        if s.devices > MAX_DEVICES {
            return Err(ProtoError::Range("devices"));
        }
        if s.fw.len() > MAX_STR {
            return Err(ProtoError::Range("str"));
        }
        return Ok(Line::Status(s));
    }
    Err(ProtoError::Schema("line"))
}

/// Decoupe le flux UART en lignes. Une ligne trop longue est jetee EN
/// ENTIER jusqu'au prochain `\n` : on ne la tronque jamais, sinon sa fin
/// serait prise pour une nouvelle ligne.
pub struct LineReader {
    buf: Vec<u8>,
    overflow: bool,
}

impl Default for LineReader {
    fn default() -> Self {
        Self::new()
    }
}

impl LineReader {
    pub fn new() -> Self {
        LineReader {
            buf: Vec::with_capacity(MAX_LINE),
            overflow: false,
        }
    }

    /// Renvoie `Some` a chaque `\n` : la ligne (sans `\n` ni `\r`), ou
    /// `TooLong` si elle a deborde.
    pub fn push(&mut self, b: u8) -> Option<Result<Vec<u8>, ProtoError>> {
        if b == b'\n' {
            let r = if self.overflow {
                Err(ProtoError::TooLong)
            } else {
                if self.buf.last() == Some(&b'\r') {
                    self.buf.pop();
                }
                Ok(std::mem::take(&mut self.buf))
            };
            self.buf.clear();
            self.overflow = false;
            return Some(r);
        }
        if self.buf.len() + 1 >= MAX_LINE {
            self.overflow = true;
        } else if !self.overflow {
            self.buf.push(b);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IEEE: &str = "0x00124b0012345678";

    #[test]
    fn ieee_roundtrip() {
        let i: Ieee = IEEE.parse().unwrap();
        assert_eq!(i, Ieee(0x00124b0012345678));
        assert_eq!(i.to_string(), IEEE);
        assert!("00124b0012345678".parse::<Ieee>().is_err());
        assert!("0x00124b00123456789".parse::<Ieee>().is_err());
        assert!("0xZZ".parse::<Ieee>().is_err());
    }

    #[test]
    fn encode_bounds() {
        assert_eq!(
            encode_request(1, &Request::Status).unwrap(),
            "{\"cmd\":\"status\",\"id\":1}\n"
        );
        assert_eq!(
            encode_request(4, &Request::PermitJoin { s: 0 }),
            Err(ProtoError::Range("s"))
        );
        assert_eq!(
            encode_request(4, &Request::PermitJoin { s: 255 }),
            Err(ProtoError::Range("s"))
        );
        assert!(encode_request(4, &Request::PermitJoin { s: 1 }).is_ok());
        assert!(encode_request(4, &Request::PermitJoin { s: 254 }).is_ok());
        assert_eq!(
            encode_request(
                6,
                &Request::ResetNetwork {
                    ch: 10,
                    pan: 0x1b00
                }
            ),
            Err(ProtoError::Range("ch"))
        );
        assert_eq!(
            encode_request(
                6,
                &Request::ResetNetwork {
                    ch: 20,
                    pan: 0xFFFF
                }
            ),
            Err(ProtoError::Range("pan"))
        );
        let r = encode_request(
            6,
            &Request::ResetNetwork {
                ch: 20,
                pan: 0x1b00,
            },
        )
        .unwrap();
        assert!(r.contains("\"pan\":\"0x1b00\"") && r.contains("\"confirm\":\"RESET\""));
        assert_eq!(
            encode_request(
                7,
                &Request::SetBlePasskey {
                    pk: "12345a".into()
                }
            ),
            Err(ProtoError::Range("pk"))
        );
        assert_eq!(
            encode_request(3, &Request::Reports { after: 0, max: 65 }),
            Err(ProtoError::Range("max"))
        );
    }

    #[test]
    fn parse_status() {
        let l = br#"{"id":1,"fw":"27.01.0","ch":15,"pan":"0x1A62","epan":"0x00124b0000000001","devices":3,"permit_join":0,"seq":10234,"dropped":0,"uptime_s":99}"#;
        match parse_line(1, l).unwrap() {
            Line::Status(s) => {
                assert_eq!(s.pan, 0x1a62);
                assert_eq!(s.devices, 3);
            }
            other => panic!("{other:?}"),
        }
        let bad_ch = br#"{"id":1,"fw":"x","ch":27,"pan":"0x1","epan":"0x1","devices":3,"permit_join":0,"seq":1,"dropped":0,"uptime_s":1}"#;
        assert_eq!(parse_line(1, bad_ch), Err(ProtoError::Range("ch")));
        let bad_dev = br#"{"id":1,"fw":"x","ch":15,"pan":"0x1","epan":"0x1","devices":33,"permit_join":0,"seq":1,"dropped":0,"uptime_s":1}"#;
        assert_eq!(parse_line(1, bad_dev), Err(ProtoError::Range("devices")));
    }

    #[test]
    fn parse_device_and_hostile_strings() {
        let ok = br#"{"id":2,"ieee":"0x00124b0012345678","nwk":"0x3c1f","mfr":"LUMI","model":"lumi.weather","power":"battery","eps":[1],"clusters":[1026,1029,1027,1],"lqi":180,"last_seen_s":42}"#;
        match parse_line(2, ok).unwrap() {
            Line::Device(d) => {
                assert_eq!(d.nwk, 0x3c1f);
                assert_eq!(d.zone_type, None);
            }
            other => panic!("{other:?}"),
        }
        // Un modele hostile mais court passe : il sera echappe a la sortie
        let xss = br#"{"id":2,"ieee":"0x00124b0012345678","nwk":"0x1","mfr":"a\"b","model":"<img src=x>","power":"mains","eps":[],"clusters":[],"lqi":1,"last_seen_s":1}"#;
        assert!(matches!(parse_line(2, xss), Ok(Line::Device(_))));
        let long = format!(
            r#"{{"id":2,"ieee":"0x1","nwk":"0x1","mfr":"{}","model":"m","power":"mains","eps":[],"clusters":[],"lqi":1,"last_seen_s":1}}"#,
            "A".repeat(33)
        );
        assert_eq!(
            parse_line(2, long.as_bytes()),
            Err(ProtoError::Range("str"))
        );
    }

    #[test]
    fn parse_report_end_and_errors() {
        let r = br#"{"id":3,"seq":10201,"ieee":"0x00124b0012345678","ep":1,"cl":1026,"at":0,"ty":41,"v":2150,"lqi":176,"age_s":122}"#;
        assert!(matches!(
            parse_line(3, r),
            Ok(Line::Report(Report {
                seq: 10201,
                cl: 1026,
                ..
            }))
        ));
        assert_eq!(
            parse_line(3, br#"{"id":3,"end":10264,"more":true}"#),
            Ok(Line::End {
                end: 10264,
                more: true
            })
        );
        assert_eq!(
            parse_line(2, br#"{"id":2,"end":3}"#),
            Ok(Line::End {
                end: 3,
                more: false
            })
        );
        assert_eq!(parse_line(4, br#"{"id":4,"ok":true}"#), Ok(Line::Ok));
        assert_eq!(
            parse_line(4, br#"{"id":4,"err":"range"}"#),
            Ok(Line::Err("range".into()))
        );
        // reponse a une autre requete, JSON tronque, champ hors type
        assert_eq!(
            parse_line(5, br#"{"id":4,"ok":true}"#),
            Err(ProtoError::WrongId)
        );
        assert_eq!(
            parse_line(3, br#"{"id":3,"seq":1,"ieee":"0x1""#),
            Err(ProtoError::Json)
        );
        let lqi_overflow =
            br#"{"id":3,"seq":1,"ieee":"0x1","ep":1,"cl":1,"at":0,"ty":0,"v":1,"lqi":300,"age_s":0}"#;
        assert_eq!(
            parse_line(3, lqi_overflow),
            Err(ProtoError::Schema("report"))
        );
        assert_eq!(parse_line(1, &[b'{'; MAX_LINE]), Err(ProtoError::TooLong));
    }

    #[test]
    fn parse_events_ignore_id() {
        assert_eq!(
            parse_line(9, br#"{"ev":"portal_request"}"#),
            Ok(Line::Event(Event::PortalRequest))
        );
        assert_eq!(
            parse_line(9, br#"{"ev":"joined","ieee":"0x00124b0012345678"}"#),
            Ok(Line::Event(Event::Joined(Ieee(0x00124b0012345678))))
        );
        assert_eq!(
            parse_line(9, br#"{"ev":"bogus"}"#),
            Err(ProtoError::Schema("ev"))
        );
    }

    #[test]
    fn line_reader_drops_overlong_and_resyncs() {
        let mut r = LineReader::new();
        let mut out = vec![];
        // log de boot ROM sans fin de ligne puis une vraie trame
        let mut stream = vec![b'x'; 700];
        stream.extend_from_slice(b"\n{\"id\":1,\"ok\":true}\r\n");
        for b in stream {
            if let Some(l) = r.push(b) {
                out.push(l);
            }
        }
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], Err(ProtoError::TooLong));
        assert_eq!(out[1].as_deref(), Ok(&br#"{"id":1,"ok":true}"#[..]));
        // 511 octets + \n = 512 : accepte ; 512 + \n : refuse
        let mut r = LineReader::new();
        for _ in 0..511 {
            assert!(r.push(b'a').is_none());
        }
        assert!(matches!(r.push(b'\n'), Some(Ok(v)) if v.len() == 511));
        for _ in 0..512 {
            r.push(b'a');
        }
        assert_eq!(r.push(b'\n'), Some(Err(ProtoError::TooLong)));
    }
}
