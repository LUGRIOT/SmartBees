# RUCHE CCA V27.01 — Plan 1 : fondations (ruche-core + spikes S1–S3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Livrer la crate `ruche-core`, c'est-à-dire toute la logique métier du P4 testée sur PC (lots L1 + L2 de la spec), et lever les trois risques matériels bloquants (spikes S1, S2, S3) avant d'écrire les firmwares.

**Architecture:** `ruche-core` est une crate Rust `std` sans aucune dépendance ESP. Elle contient le protocole UART C6, les converters ZCL façon Zigbee2MQTT, l'état du pont, l'analyse audio, la machine d'alarme, les réglages et les payloads MQTT. Le firmware P4 (plan 3) ne fera que brancher les drivers dessus. Les spikes sont des prototypes jetables sur le matériel réel, et chacun se conclut par une note go / no-go qui conditionne les plans 2 à 4.

**Tech Stack:** Rust stable 1.94 (tests sur PC), édition 2021, `serde 1.0.229`, `serde_json 1.0.154`, `microfft 0.6.0`. Pour les spikes : ESP-IDF v5.5, `espup`/`espflash`, `esp-idf-svc`, `esp_modem`, `esp-zigbee-lib ~1.6.0`, NimBLE.

**Spec:** `docs/superpowers/specs/2026-10-21-ruche-cca-v27-01-design.md`, rev 1.1. À lire avant de commencer : §2.4 (protocole), §2.5.3 (ruche-core), §2.6 (payloads), §2.9 (réglages), §3.3 (spikes).

**Découpage en plans :** la spec couvre trois sous-systèmes. Ce plan-ci ne dépend d'aucun résultat de spike. Les suivants seront écrits **après** les notes de spike, parce que leurs API dépendent de ce que S1 à S3 auront validé :
- Plan 2 : firmware C6 (L3, et la partie C6 de L7).
- Plan 3 : firmware P4, drivers et cycle (L4 à L6).
- Plan 4 : portail, OTA et release (partie P4 de L7, L8).

> Le firmware v3 (`cca-ruche/`, C / ESP-IDF v5.5) n'est **pas publié** dans ce dépôt : il contient des failles connues (XSS du tableau de bord, API sans authentification, mesh UDP non authentifié) et la V27 le remplace. Les mentions de ses fichiers renvoient à la copie privée du mainteneur ; les valeurs utiles (délais du modem, broches SD, sdkconfig) sont reprises ici.

## Global Constraints

- Branche de travail : `RUCHE-CCA-V27.01`. Ne jamais commiter de secret (`*.key` est déjà dans `.gitignore`).
- `ruche-core` : **aucune dépendance ESP**. Seules dépendances autorisées : `serde`, `serde_json`, `microfft`. Pas de `heapless` (la spec le citait, mais il n'est pas nécessaire en `std`). Rien d'autre sans justification écrite.
- Chemin de la crate : `firmware-p4/ruche-core/`. Le workspace et `ruche-fw` viendront dans le plan 3.
- Trame UART : **512 octets maximum, `\n` compris** ; chaînes venant d'un appareil : 32 octets maximum ; 32 appareils maximum.
- Permit join : 1 à 254 s. Canal : 11 à 26. PAN : ni `0x0000` ni `0xFFFF`. Passkey BLE : 6 chiffres.
- Friendly names et HIVE_ID : `[A-Za-z0-9_-]{1,32}`.
- Topics : `ruche/<HIVE_ID>/{data, zigbee/<friendly_name>, bridge/state, bridge/devices, bridge/event, alarm}`.
- `data` garde les champs v3 `hive_id, timestamp, temperature (1 déc.), humidity (0), pressure (0), weight_kg (2), sound_hz (0), db_level (1)`. `satellites` est retiré.
- Audio : 16 kHz, trames de 4096 échantillons (256 ms), bandes 100–200 / 200–300 / 300–500 / 500–1000 Hz, fréquence dominante cherchée entre 50 et 2000 Hz, `spl_offset_db` par défaut 120.
- Réglages : valeurs par défaut et bornes exactement celles de spec §2.9.
- Avant chaque commit : `cargo test`, `cargo clippy --all-targets -- -D warnings` et `cargo fmt --check` passent dans `firmware-p4/ruche-core`.
- Commentaires en français, sans accents dans le code (style v3), et qui expliquent le **pourquoi**.
- Messages de commit au format Conventional Commits, terminés par `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **Ligne UART parasite ou trop longue** (log de la ROM au reset du C6, ligne coupée) : elle doit être jetée entière, et la trame valide suivante doit être lue normalement. Testé par `line_reader_drops_overlong_and_resyncs` (Task 1).
2. **Report d'un appareil absent de la table** (appairé pendant le sommeil du P4, table pas encore relue) : il doit être converti avec les règles génériques, pas perdu. Testé par `unknown_device_still_converted` (Task 5).
3. **Renommage vers l'adresse IEEE d'un autre appareil** encore non renommé : il doit être refusé, sinon deux appareils publieraient sur le même topic. Testé par `rename_uniqueness_on_effective_names` (Task 3).
4. **Fabricant ou modèle hostile** (`"`, `<script>`) : il doit rester du JSON valide et être restitué tel quel ; au-delà de 32 octets, il est refusé. Testé par `hostile_device_strings_are_escaped` (Task 8) et `parse_device_and_hostile_strings` (Task 1).
5. **Horloge non synchronisée, ou mesure de poids périmée** : `timestamp` doit valoir null et `uptime_ms` être présent ; un report de batterie ne doit pas rafraîchir l'âge du poids. Testé par `data_nulls_when_missing` (Task 8) et `weight_selection_and_staleness` (Task 5).

---

### Task 1: Crate `ruche-core` + protocole UART C6 (`c6_proto`)

**Files:**
- Create: `firmware-p4/ruche-core/Cargo.toml`
- Create: `firmware-p4/ruche-core/src/lib.rs`
- Create: `firmware-p4/ruche-core/src/c6_proto.rs`
- Create: `firmware-p4/ruche-core/Cargo.lock` (généré, à commiter)

**Interfaces:**
- Consumes: rien.
- Produces :
  - constantes `MAX_LINE = 512`, `MAX_STR = 32`, `MAX_DEVICES = 32` ;
  - `struct Ieee(pub u64)` : `Display`/`FromStr` au format `0x%016x`, `Serialize`/`Deserialize` en chaîne, `Ord` ;
  - `enum Request { Status, Devices, Reports{after:u32,max:u16}, PermitJoin{s:u8}, Remove{ieee:Ieee}, ResetNetwork{ch:u8,pan:u16}, SetBlePasskey{pk:String} }` ;
  - `fn encode_request(id:u16, &Request) -> Result<String, ProtoError>` ;
  - `fn parse_line(expected_id:u16, &[u8]) -> Result<Line, ProtoError>` ;
  - `enum Line { Status, Device, Report, End{end,more}, Ok, Err(String), Event }` ;
  - les structures `Status`, `DeviceInfo{ieee,nwk,mfr,model,power,eps,clusters,lqi,last_seen_s,zone_type,analog_units}`, `Report{seq,ieee,ep,cl,at,ty,v:f64,lqi,age_s}` et `enum Power`, `enum Event` ;
  - `LineReader::new()` et `.push(u8) -> Option<Result<Vec<u8>, ProtoError>>` ;
  - `enum ProtoError { TooLong, Json, Schema(&str), WrongId, Range(&str) }`.

> Écart avec spec §2.4, à reporter dans la spec au plan 2 : `devices` porte en plus `zone_type` et `analog_units`, deux champs optionnels lus par le C6 à l'interview. Ils servent à convertir les zones IAS et la pesée (§2.3.2).

- [ ] **Step 1: Créer la crate et écrire les tests**

`firmware-p4/ruche-core/Cargo.toml` :

```toml
[package]
name = "ruche-core"
version = "0.1.0"
edition = "2021"
description = "Logique metier RUCHE CCA V27, sans dependance ESP"

[dependencies]
microfft = "0.6.0"
serde = { version = "1.0.229", features = ["derive"] }
serde_json = "1.0.154"
```

`firmware-p4/ruche-core/src/lib.rs` :

```rust
//! Logique metier de la ruche V27, sans aucune dependance ESP :
//! tout ce qui est ici se teste sur PC avec `cargo test`.
pub mod c6_proto;
```

`firmware-p4/ruche-core/src/c6_proto.rs` : pour l'instant, uniquement le module de tests.

```rust
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
```

- [ ] **Step 2: Vérifier que les tests échouent**

Run: `cargo test --manifest-path firmware-p4/ruche-core/Cargo.toml c6_proto`
Expected: la compilation échoue avec `cannot find type Ieee` / `cannot find function parse_line` (E0412, E0425).

- [ ] **Step 3: Écrire l'implémentation**

Insérer en **tête** de `src/c6_proto.rs`, au-dessus de `#[cfg(test)]` :

```rust
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
```

- [ ] **Step 4: Vérifier que les tests passent**

Run: `cd firmware-p4/ruche-core && cargo fmt && cargo test c6_proto && cargo clippy --all-targets -- -D warnings`
Expected: `test result: ok. 7 passed`, sans aucun avertissement.

- [ ] **Step 5: Commit**

```bash
git add firmware-p4/ruche-core/Cargo.toml firmware-p4/ruche-core/Cargo.lock firmware-p4/ruche-core/src
git commit -m "feat(core): protocole UART P4<->C6 v2 (trames bornees, schema valide)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Converters ZCL (`zcl`)

**Files:**
- Create: `firmware-p4/ruche-core/src/zcl.rs`
- Modify: `firmware-p4/ruche-core/src/lib.rs` (ajouter `pub mod zcl;`)

**Interfaces:**
- Consumes: rien.
- Produces :
  - `const WEIGHT_MODEL = "CCA-RUCHE-WEIGHT"`, `const UNITS_KG: u16 = 39` ;
  - `struct Ctx<'a> { model:&str, zone_type:Option<u16>, analog_units:Option<u16> }` (`Default`) ;
  - `fn convert(cluster:u16, attr:u16, raw:f64, &Ctx) -> Vec<(&'static str, serde_json::Value)>` ;
  - `fn exposes(&[u16], &Ctx) -> Vec<&'static str>`.

Les noms de propriétés (`temperature`, `humidity`, `pressure`, `illuminance`, `battery`, `voltage`, `occupancy`, `state`, `co2`, `pm25`, `contact`, `water_leak`, `alarm`, `battery_low`, `weight`, `analog_value`) sont **ceux de Zigbee2MQTT**, pour que la comparaison T-INT-04 soit directe.

- [ ] **Step 1: Écrire les tests**

Ajouter `pub mod zcl;` dans `src/lib.rs`, puis créer `src/zcl.rs` avec :

```rust
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
```

- [ ] **Step 2: Vérifier que les tests échouent**

Run: `cargo test --manifest-path firmware-p4/ruche-core/Cargo.toml zcl`
Expected: la compilation échoue (`cannot find function convert`, `cannot find struct Ctx`).

- [ ] **Step 3: Écrire l'implémentation**

En tête de `src/zcl.rs` :

```rust
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
```

- [ ] **Step 4: Vérifier que les tests passent**

Run: `cd firmware-p4/ruche-core && cargo fmt && cargo test zcl && cargo clippy --all-targets -- -D warnings`
Expected: `5 passed`.

- [ ] **Step 5: Commit**

```bash
git add firmware-p4/ruche-core/src/zcl.rs firmware-p4/ruche-core/src/lib.rs
git commit -m "feat(core): converters ZCL standard facon Zigbee2MQTT (+ pesee Analog Input)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Friendly names (`names`)

**Files:**
- Create: `firmware-p4/ruche-core/src/names.rs`
- Modify: `firmware-p4/ruche-core/src/lib.rs` (ajouter `pub mod names;`)

**Interfaces:**
- Consumes: `c6_proto::Ieee` (Task 1).
- Produces :
  - `enum NameError { Length, Charset, Duplicate }` ;
  - `fn validate(&str) -> Result<(), NameError>` ;
  - `fn effective(&BTreeMap<Ieee,String>, Ieee) -> String` ;
  - `fn check_rename(known:&[Ieee], names:&BTreeMap<Ieee,String>, ieee:Ieee, new:&str) -> Result<(), NameError>`.

- [ ] **Step 1: Écrire les tests**

Ajouter `pub mod names;` dans `src/lib.rs`, puis créer `src/names.rs` avec :

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charset_and_length() {
        assert_eq!(validate("meteo_rucher-1"), Ok(()));
        for bad in ["a/b", "a+b", "a#b", "a b", "é", "a.b"] {
            assert_eq!(validate(bad), Err(NameError::Charset), "{bad}");
        }
        assert_eq!(validate(""), Err(NameError::Length));
        assert_eq!(validate(&"a".repeat(33)), Err(NameError::Length));
        assert_eq!(validate(&"a".repeat(32)), Ok(()));
    }

    #[test]
    fn rename_uniqueness_on_effective_names() {
        let a = Ieee(1);
        let b = Ieee(2);
        let mut names = BTreeMap::new();
        names.insert(a, "meteo".to_string());
        let known = [a, b];
        assert_eq!(
            check_rename(&known, &names, b, "meteo"),
            Err(NameError::Duplicate)
        );
        // le nom par defaut de b est son adresse
        assert_eq!(
            check_rename(&known, &names, a, &b.to_string()),
            Err(NameError::Duplicate)
        );
        // se renommer soi-meme a l'identique est permis
        assert_eq!(check_rename(&known, &names, a, "meteo"), Ok(()));
        assert_eq!(effective(&names, b), "0x0000000000000002");
    }
}
```

- [ ] **Step 2: Vérifier que les tests échouent**

Run: `cargo test --manifest-path firmware-p4/ruche-core/Cargo.toml names`
Expected: la compilation échoue (`cannot find function validate`).

- [ ] **Step 3: Écrire l'implémentation**

En tête de `src/names.rs` :

```rust
//! Friendly names des appareils Zigbee. Ils finissent dans un topic MQTT
//! (`ruche/<HIVE_ID>/zigbee/<nom>`) : `/`, `+` et `#` y sont interdits
//! (NFR-SE-04). La meme regle sert pour HIVE_ID.
use crate::c6_proto::Ieee;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum NameError {
    Length,
    Charset,
    Duplicate,
}

/// `[A-Za-z0-9_-]{1,32}`
pub fn validate(name: &str) -> Result<(), NameError> {
    if name.is_empty() || name.len() > 32 {
        return Err(NameError::Length);
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(NameError::Charset);
    }
    Ok(())
}

/// Nom effectif : le friendly name, sinon l'adresse IEEE (comme Z2M).
pub fn effective(names: &BTreeMap<Ieee, String>, ieee: Ieee) -> String {
    names
        .get(&ieee)
        .cloned()
        .unwrap_or_else(|| ieee.to_string())
}

/// Verifie un renommage. L'unicite porte sur les noms EFFECTIFS : appeler
/// un appareil « 0x… » comme l'adresse d'un autre appareil non renomme
/// ferait publier les deux sur le meme topic.
pub fn check_rename(
    known: &[Ieee],
    names: &BTreeMap<Ieee, String>,
    ieee: Ieee,
    new: &str,
) -> Result<(), NameError> {
    validate(new)?;
    let clash = known
        .iter()
        .any(|&other| other != ieee && effective(names, other) == new);
    if clash {
        return Err(NameError::Duplicate);
    }
    Ok(())
}
```

- [ ] **Step 4: Vérifier que les tests passent**

Run: `cd firmware-p4/ruche-core && cargo fmt && cargo test names && cargo clippy --all-targets -- -D warnings`
Expected: `2 passed`.

- [ ] **Step 5: Commit**

```bash
git add firmware-p4/ruche-core/src/names.rs firmware-p4/ruche-core/src/lib.rs
git commit -m "feat(core): friendly names surs pour les topics MQTT (charset, unicite)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Réglages et bornes (`config`)

**Files:**
- Create: `firmware-p4/ruche-core/src/config.rs`
- Modify: `firmware-p4/ruche-core/src/lib.rs` (ajouter `pub mod config;`)

**Interfaces:**
- Consumes: `c6_proto::Ieee` (Task 1).
- Produces :
  - `const ALLOWED_INTERVALS_S: [u32; 10]` ;
  - `enum RecMode { Off, OnAlarm, Always }` (serde `snake_case`) ;
  - `struct Settings` : champs de spec §2.9, `Default`, `#[serde(default)]` pour accepter un JSON partiel ;
  - `Settings::validate(&self, sd_capacity_mb: Option<u32>) -> Result<(), ConfigError>` ;
  - `struct ConfigError(pub &'static str)`, qui contient le nom du champ fautif.

- [ ] **Step 1: Écrire les tests**

Ajouter `pub mod config;` dans `src/lib.rs`, puis créer `src/config.rs` avec :

```rust
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
```

- [ ] **Step 2: Vérifier que les tests échouent**

Run: `cargo test --manifest-path firmware-p4/ruche-core/Cargo.toml config`
Expected: la compilation échoue (`cannot find struct Settings`).

- [ ] **Step 3: Écrire l'implémentation**

En tête de `src/config.rs` :

```rust
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

impl Settings {
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
        let quota_max = sd_capacity_mb.map_or(u32::MAX, |c| (c as u64 * 9 / 10) as u32);
        within(self.rec_quota_mb, 64, quota_max, "rec_quota_mb")?;
        within(self.portal_idle_min, 2, 60, "portal_idle_min")?;
        within(self.weight_max_age_s, 600, 86400, "weight_max_age_s")?;
        Ok(())
    }
}
```

- [ ] **Step 4: Vérifier que les tests passent**

Run: `cd firmware-p4/ruche-core && cargo fmt && cargo test config && cargo clippy --all-targets -- -D warnings`
Expected: `3 passed`.

- [ ] **Step 5: Commit**

```bash
git add firmware-p4/ruche-core/src/config.rs firmware-p4/ruche-core/src/lib.rs
git commit -m "feat(core): reglages V27 avec bornes et valeurs par defaut (spec 2.9)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: État du pont Zigbee (`bridge`)

**Files:**
- Create: `firmware-p4/ruche-core/src/bridge.rs`
- Modify: `firmware-p4/ruche-core/src/lib.rs` (ajouter `pub mod bridge;`)

**Interfaces:**
- Consumes: `c6_proto::{DeviceInfo, Ieee, Report, Power}` (Task 1) ; `zcl::{convert, Ctx, WEIGHT_MODEL}` (Task 2).
- Produces :
  - `struct Stamped { v: Value, ts: u64 }` ;
  - `struct DeviceState { props: BTreeMap<String, Stamped>, lqi: u8, last_seen: u64 }` ;
  - `struct Bridge { pub devices: BTreeMap<Ieee, DeviceInfo>, pub states: BTreeMap<Ieee, DeviceState>, .. }` (`Default`, `Serialize`, `Deserialize`) ;
  - méthodes de `Bridge` : `set_devices(Vec<DeviceInfo>)`, `apply(&Report, now:u64)`, `take_updated() -> Vec<Ieee>`, `weight_kg(chosen:Option<Ieee>, now:u64, max_age_s:u32) -> Option<f64>`.

`now` est l'horloge RTC du P4 en secondes. Elle continue de tourner pendant le deep sleep, et SNTP la recale. Le firmware (plan 3) sérialise `Bridge` dans une mémoire qui survit au deep sleep.

- [ ] **Step 1: Écrire les tests**

Ajouter `pub mod bridge;` dans `src/lib.rs`, puis créer `src/bridge.rs` avec :

```rust
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
```

- [ ] **Step 2: Vérifier que les tests échouent**

Run: `cargo test --manifest-path firmware-p4/ruche-core/Cargo.toml bridge`
Expected: la compilation échoue (`cannot find struct Bridge`).

- [ ] **Step 3: Écrire l'implémentation**

En tête de `src/bridge.rs` :

```rust
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
```

- [ ] **Step 4: Vérifier que les tests passent**

Run: `cd firmware-p4/ruche-core && cargo fmt && cargo test bridge && cargo clippy --all-targets -- -D warnings`
Expected: `5 passed`.

- [ ] **Step 5: Commit**

```bash
git add firmware-p4/ruche-core/src/bridge.rs firmware-p4/ruche-core/src/lib.rs
git commit -m "feat(core): etat du pont Zigbee (derniere valeur, selection du poids)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Analyse acoustique (`audio`)

**Files:**
- Create: `firmware-p4/ruche-core/src/audio.rs`
- Modify: `firmware-p4/ruche-core/src/lib.rs` (ajouter `pub mod audio;`)

**Interfaces:**
- Consumes: `config::Settings` (Task 4).
- Produces :
  - constantes `SAMPLE_RATE: f32 = 16000.0`, `FRAME: usize = 4096`, `BANDS_HZ` ;
  - `struct FrameMetrics { db_spl, f_dom_hz, bands:[f32;4], silent }` et sa méthode `.ratio_b3_b2()` ;
  - `enum Rule { LvlMax, LvlMin, FreqMax, FreqMin, Ratio }` (serde `snake_case`) ;
  - `fn analyze_frame(&[i32; FRAME], spl_offset_db:f32) -> FrameMetrics` ;
  - `fn frame_breach(&FrameMetrics, &Settings) -> Option<Rule>` ;
  - `WindowAcc` (`Default`), avec `push(&FrameMetrics, &Settings)` et `finish(self, &Settings) -> WindowSummary` ;
  - `struct WindowSummary { frames:u16, db, f_dom, bands, ratio_b3_b2, breach:Option<Rule>, mic_fault }`.

Les échantillons attendus sont en **24 bits signés**, c'est-à-dire le mot I2S 32 bits de l'INMP441 décalé de 8 bits (`>> 8`). La normalisation en dBFS est donc équivalente au calcul de la v3 (`sensors.c`, `rms / 2^31`).

- [ ] **Step 1: Écrire les tests**

Ajouter `pub mod audio;` dans `src/lib.rs`, puis créer `src/audio.rs` avec :

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Sinus d'amplitude `amp` (fraction de la pleine echelle).
    fn sine(hz: f32, amp: f32) -> [i32; FRAME] {
        let mut out = [0i32; FRAME];
        for (i, o) in out.iter_mut().enumerate() {
            let t = i as f32 / SAMPLE_RATE;
            *o = (amp * FULL_SCALE * (2.0 * core::f32::consts::PI * hz * t).sin()) as i32;
        }
        out
    }

    #[test]
    fn dominant_frequency_within_8hz() {
        for hz in [150.0, 250.0, 450.0, 777.0] {
            let m = analyze_frame(&sine(hz, 0.1), 120.0);
            assert!((m.f_dom_hz - hz).abs() <= 8.0, "{hz} -> {}", m.f_dom_hz);
        }
    }

    #[test]
    fn level_in_db() {
        // sinus de RMS 0,5 pleine echelle = -6,02 dBFS
        let m = analyze_frame(&sine(250.0, 0.5 * core::f32::consts::SQRT_2), 120.0);
        assert!((m.db_spl - (120.0 - 6.02)).abs() < 0.1, "{}", m.db_spl);
    }

    #[test]
    fn bands_and_ratio() {
        let normal = analyze_frame(&sine(250.0, 0.1), 120.0);
        assert!(normal.bands[1] > 0.9, "{:?}", normal.bands);
        let piping = analyze_frame(&sine(450.0, 0.1), 120.0);
        assert!(piping.ratio_b3_b2() > 1.5);
        assert!((piping.bands.iter().sum::<f32>() - 1.0).abs() < 1e-3);
    }

    #[test]
    fn silence_is_mic_fault() {
        let m = analyze_frame(&[0; FRAME], 120.0);
        assert!(m.silent);
        let s = Settings::default();
        let mut w = WindowAcc::default();
        for _ in 0..10 {
            w.push(&m, &s);
        }
        let sum = w.finish(&s);
        assert!(sum.mic_fault);
        assert_eq!(sum.breach, None);
    }

    #[test]
    fn rules_in_order() {
        let s = Settings::default();
        let base = FrameMetrics {
            db_spl: 60.0,
            f_dom_hz: 250.0,
            bands: [0.1, 0.6, 0.2, 0.1],
            silent: false,
        };
        assert_eq!(frame_breach(&base, &s), None);
        assert_eq!(
            frame_breach(
                &FrameMetrics {
                    db_spl: 90.0,
                    f_dom_hz: 450.0,
                    ..base
                },
                &s
            ),
            Some(Rule::LvlMax)
        );
        assert_eq!(
            frame_breach(
                &FrameMetrics {
                    db_spl: 30.0,
                    ..base
                },
                &s
            ),
            Some(Rule::LvlMin)
        );
        assert_eq!(
            frame_breach(
                &FrameMetrics {
                    f_dom_hz: 450.0,
                    ..base
                },
                &s
            ),
            Some(Rule::FreqMax)
        );
        assert_eq!(
            frame_breach(
                &FrameMetrics {
                    f_dom_hz: 100.0,
                    ..base
                },
                &s
            ),
            Some(Rule::FreqMin)
        );
        assert_eq!(
            frame_breach(
                &FrameMetrics {
                    bands: [0.1, 0.3, 0.5, 0.1],
                    ..base
                },
                &s
            ),
            Some(Rule::Ratio)
        );
    }

    #[test]
    fn window_breach_ratio() {
        let s = Settings::default(); // breach_ratio 0,6
        let ok = FrameMetrics {
            db_spl: 60.0,
            f_dom_hz: 250.0,
            bands: [0.1, 0.6, 0.2, 0.1],
            silent: false,
        };
        let bad = FrameMetrics {
            f_dom_hz: 450.0,
            ..ok
        };
        let run = |n_bad: usize| {
            let mut w = WindowAcc::default();
            for i in 0..10 {
                w.push(if i < n_bad { &bad } else { &ok }, &s);
            }
            w.finish(&s)
        };
        assert_eq!(run(5).breach, None);
        assert_eq!(run(6).breach, Some(Rule::FreqMax));
        assert_eq!(run(6).f_dom, 450.0);
        assert_eq!(run(0).frames, 10);
    }
}
```

- [ ] **Step 2: Vérifier que les tests échouent**

Run: `cargo test --manifest-path firmware-p4/ruche-core/Cargo.toml audio`
Expected: la compilation échoue (`cannot find function analyze_frame`).

- [ ] **Step 3: Écrire l'implémentation**

En tête de `src/audio.rs` :

```rust
//! Analyse acoustique (spec §2.5.3, FR-AU-02/03) : trames de 4096
//! echantillons a 16 kHz (256 ms), fenetre de Hann, FFT reelle.
//! Les echantillons arrivent deja ramenes a 24 bits signes (INMP441,
//! mot I2S de 32 bits decale de 8).
use crate::config::Settings;
use serde::Serialize;

pub const SAMPLE_RATE: f32 = 16_000.0;
pub const FRAME: usize = 4096;
/// Pleine echelle d'un echantillon 24 bits.
const FULL_SCALE: f32 = 8_388_608.0;
const BIN_HZ: f32 = SAMPLE_RATE / FRAME as f32; // 3,906 Hz
pub const BANDS_HZ: [(f32, f32); 4] = [
    (100.0, 200.0),
    (200.0, 300.0),
    (300.0, 500.0),
    (500.0, 1000.0),
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameMetrics {
    pub db_spl: f32,
    pub f_dom_hz: f32,
    /// Part de l'energie 100-1000 Hz dans chaque bande (somme = 1)
    pub bands: [f32; 4],
    pub silent: bool,
}

impl FrameMetrics {
    pub fn ratio_b3_b2(&self) -> f32 {
        if self.bands[1] > 0.0 {
            self.bands[2] / self.bands[1]
        } else {
            0.0
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    LvlMax,
    LvlMin,
    FreqMax,
    FreqMin,
    Ratio,
}

pub fn analyze_frame(samples: &[i32; FRAME], spl_offset_db: f32) -> FrameMetrics {
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    if sum_sq == 0.0 {
        return FrameMetrics {
            db_spl: 0.0,
            f_dom_hz: 0.0,
            bands: [0.0; 4],
            silent: true,
        };
    }
    let rms = (sum_sq / FRAME as f64).sqrt() as f32;
    let db_spl = 20.0 * (rms / FULL_SCALE).log10() + spl_offset_db;

    let mut buf = [0f32; FRAME];
    for (i, (b, &s)) in buf.iter_mut().zip(samples.iter()).enumerate() {
        let hann = 0.5 - 0.5 * (2.0 * core::f32::consts::PI * i as f32 / FRAME as f32).cos();
        *b = s as f32 / FULL_SCALE * hann;
    }
    let spec = microfft::real::rfft_4096(&mut buf);
    // bin 0 : DC (re) et Nyquist (im) empaquetes -> ignore
    let power = |k: usize| spec[k].norm_sqr();

    let bin = |hz: f32| (hz / BIN_HZ).round() as usize;
    let (mut peak_k, mut peak_p) = (0, 0f32);
    for k in bin(50.0)..=bin(2000.0) {
        if power(k) > peak_p {
            peak_k = k;
            peak_p = power(k);
        }
    }

    let mut bands = [0f32; 4];
    for (e, (lo, hi)) in bands.iter_mut().zip(BANDS_HZ) {
        *e = (bin(lo)..bin(hi)).map(power).sum();
    }
    let total: f32 = bands.iter().sum();
    if total > 0.0 {
        bands.iter_mut().for_each(|e| *e /= total);
    }

    FrameMetrics {
        db_spl,
        f_dom_hz: peak_k as f32 * BIN_HZ,
        bands,
        silent: false,
    }
}

/// Premiere regle de la consigne enfreinte par la trame (FR-AU-03).
pub fn frame_breach(m: &FrameMetrics, s: &Settings) -> Option<Rule> {
    if m.silent {
        return None;
    }
    if m.db_spl > s.lvl_max_db {
        Some(Rule::LvlMax)
    } else if m.db_spl < s.lvl_min_db {
        Some(Rule::LvlMin)
    } else if m.f_dom_hz > s.f_max_hz {
        Some(Rule::FreqMax)
    } else if m.f_dom_hz < s.f_min_hz {
        Some(Rule::FreqMin)
    } else if m.ratio_b3_b2() > s.ratio_b3_b2_max {
        Some(Rule::Ratio)
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WindowSummary {
    pub frames: u16,
    pub db: f32,
    pub f_dom: f32,
    pub bands: [f32; 4],
    pub ratio_b3_b2: f32,
    /// Regle la plus souvent enfreinte, si la fenetre est hors consigne
    pub breach: Option<Rule>,
    pub mic_fault: bool,
}

/// Cumule les trames d'une fenetre d'ecoute.
#[derive(Default)]
pub struct WindowAcc {
    db: Vec<f32>,
    f_dom: Vec<f32>,
    bands: [f32; 4],
    rules: Vec<Rule>,
    frames: u16,
}

impl WindowAcc {
    pub fn push(&mut self, m: &FrameMetrics, s: &Settings) {
        self.frames += 1;
        if m.silent {
            return;
        }
        self.db.push(m.db_spl);
        self.f_dom.push(m.f_dom_hz);
        for (a, b) in self.bands.iter_mut().zip(m.bands) {
            *a += b;
        }
        if let Some(r) = frame_breach(m, s) {
            self.rules.push(r);
        }
    }

    pub fn finish(mut self, s: &Settings) -> WindowSummary {
        let n = self.db.len();
        if n == 0 {
            // micro debranche : pas d'information, donc pas d'alarme
            return WindowSummary {
                frames: self.frames,
                db: 0.0,
                f_dom: 0.0,
                bands: [0.0; 4],
                ratio_b3_b2: 0.0,
                breach: None,
                mic_fault: true,
            };
        }
        let db = self.db.iter().sum::<f32>() / n as f32;
        self.f_dom.sort_by(f32::total_cmp);
        let f_dom = self.f_dom[n / 2]; // mediane : robuste a un bruit isole
        let bands = self.bands.map(|b| b / n as f32);
        let ratio_b3_b2 = if bands[1] > 0.0 {
            bands[2] / bands[1]
        } else {
            0.0
        };
        let breach = if self.rules.len() as f32 >= s.breach_ratio * self.frames as f32 {
            [
                Rule::LvlMax,
                Rule::LvlMin,
                Rule::FreqMax,
                Rule::FreqMin,
                Rule::Ratio,
            ]
            .into_iter()
            .max_by_key(|r| self.rules.iter().filter(|x| *x == r).count())
        } else {
            None
        };
        WindowSummary {
            frames: self.frames,
            db,
            f_dom,
            bands,
            ratio_b3_b2,
            breach,
            mic_fault: false,
        }
    }
}
```

- [ ] **Step 4: Vérifier que les tests passent**

Run: `cd firmware-p4/ruche-core && cargo fmt && cargo test audio && cargo clippy --all-targets -- -D warnings`
Expected: `6 passed`.

- [ ] **Step 5: Commit**

```bash
git add firmware-p4/ruche-core/src/audio.rs firmware-p4/ruche-core/src/lib.rs
git commit -m "feat(core): analyse acoustique FFT 4096 (niveau, frequence dominante, 4 bandes)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Machine d'alarme (`alarm`)

**Files:**
- Create: `firmware-p4/ruche-core/src/alarm.rs`
- Modify: `firmware-p4/ruche-core/src/lib.rs` (ajouter `pub mod alarm;`)

**Interfaces:**
- Consumes: `config::Settings` (Task 4).
- Produces :
  - `enum Phase { Normal, Suspect(u8), Alarm, Recovering(u8) }` ;
  - `enum AlarmEvent { Raised, Active, Cleared }` (serde `snake_case`) ;
  - `struct AlarmState { pub phase, pub since: Option<u64>, pub streak: u32, .. }` (`Default`, `Serialize`, `Deserialize`) ;
  - `AlarmState::step(&mut self, breach:bool, mic_fault:bool, now:u64, &Settings) -> Option<AlarmEvent>`.

- [ ] **Step 1: Écrire les tests**

Ajouter `pub mod alarm;` dans `src/lib.rs`, puis créer `src/alarm.rs` avec :

```rust
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
```

- [ ] **Step 2: Vérifier que les tests échouent**

Run: `cargo test --manifest-path firmware-p4/ruche-core/Cargo.toml alarm`
Expected: la compilation échoue (`cannot find struct AlarmState`).

- [ ] **Step 3: Écrire l'implémentation**

En tête de `src/alarm.rs` :

```rust
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
```

- [ ] **Step 4: Vérifier que les tests passent**

Run: `cd firmware-p4/ruche-core && cargo fmt && cargo test alarm && cargo clippy --all-targets -- -D warnings`
Expected: `5 passed`.

- [ ] **Step 5: Commit**

```bash
git add firmware-p4/ruche-core/src/alarm.rs firmware-p4/ruche-core/src/lib.rs
git commit -m "feat(core): machine d'alarme acoustique (confirmation, hysteresis, cooldown)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: Payloads et topics MQTT (`payload`)

**Files:**
- Create: `firmware-p4/ruche-core/src/payload.rs`
- Modify: `firmware-p4/ruche-core/src/lib.rs` (ajouter `pub mod payload;`)

**Interfaces:**
- Consumes :
  - `alarm::AlarmEvent` (Task 7) ;
  - `audio::{WindowSummary, Rule, FRAME, SAMPLE_RATE}` (Task 6) ;
  - `bridge::Bridge` (Task 5) ;
  - `c6_proto::{Event, Ieee, DeviceInfo, Power, Report}` (Task 1) ;
  - `names::effective` (Task 3) ;
  - `zcl::{exposes, Ctx}` (Task 2).
- Produces :
  - `fn topic(hive_id:&str, suffix:&str) -> String` ;
  - `struct Env { temperature, humidity, pressure }` ;
  - `struct DataInputs<'a> { hive_id, timestamp:Option<u64>, uptime_ms, fw, env:Option<Env>, weight_kg:Option<f64>, audio:Option<&WindowSummary>, audio_state:&str, zigbee_online, zigbee_devices, zigbee_available }` ;
  - `fn data(&DataInputs) -> String` ;
  - `fn device(&Bridge, Ieee) -> Option<String>` ;
  - `fn bridge_devices(&Bridge, &BTreeMap<Ieee,String>) -> String` ;
  - `fn bridge_event(&Event) -> Option<String>` ;
  - `struct Errors { uart, lte, sd }`, `struct BridgeState<'a> { fw_p4, fw_c6, uptime_s, reset_reason, err, queue, zb_dropped }` ;
  - `fn bridge_state(&BridgeState) -> String`, `fn bridge_offline() -> String` ;
  - `fn alarm(AlarmEvent, since:Option<u64>, &WindowSummary, windows:u32, recording:Option<&str>) -> String`.

- [ ] **Step 1: Écrire les tests**

Ajouter `pub mod payload;` dans `src/lib.rs`, puis créer `src/payload.rs` avec :

```rust
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
```

- [ ] **Step 2: Vérifier que les tests échouent**

Run: `cargo test --manifest-path firmware-p4/ruche-core/Cargo.toml payload`
Expected: la compilation échoue (`cannot find function data`).

- [ ] **Step 3: Écrire l'implémentation**

En tête de `src/payload.rs` :

```rust
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
    m.insert(
        "sound_hz".into(),
        d.audio.map_or(Value::Null, |a| r(a.f_dom as f64, 0)),
    );
    m.insert(
        "db_level".into(),
        d.audio.map_or(Value::Null, |a| r(a.db as f64, 1)),
    );
    m.insert(
        "audio".into(),
        json!({
            "bands": d.audio.map(|a| a.bands.map(|b| r(b as f64, 2))),
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
```

- [ ] **Step 4: Vérifier que toute la crate passe**

Run: `cd firmware-p4/ruche-core && cargo fmt && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: `test result: ok. 38 passed; 0 failed`.

- [ ] **Step 5: Commit**

```bash
git add firmware-p4/ruche-core/src/payload.rs firmware-p4/ruche-core/src/lib.rs
git commit -m "feat(core): payloads MQTT V27 (data compatible v3, zigbee/*, bridge/*, alarm)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: Spike S1 — Rust std sur ESP32-P4 (matériel requis)

Ce spike répond à une question, il ne produit pas de code à garder. Il faut un ESP32-P4-NANO, une carte SD, l'INMP441 câblé (GPIO20/21/22), un fil de cavalier et un ampèremètre USB.

**Files:**
- Create: `spikes/s1-p4-rust/` (généré, **jetable**, commité pour référence)
- Create: `docs/spikes/S1-p4-rust.md`

**Interfaces:**
- Produces: une note go / no-go, les versions exactes de la toolchain qui fonctionnent, la consommation pendant la capture audio, la taille du binaire, et la confirmation ou non du réveil par GPIO5.

- [ ] **Step 1: Installer la toolchain Rust ESP**

```bash
cargo install espup espflash ldproxy cargo-generate --locked
espup install
. ~/export-esp.sh
rustc +esp --version
```

Expected: `rustc +esp` affiche une version. La noter.

- [ ] **Step 2: Générer le projet et flasher un « hello »**

```bash
mkdir -p spikes && cd spikes
cargo generate esp-rs/esp-idf-template cargo --name s1-p4-rust
```

Choisir `esp32p4` s'il est proposé. Sinon, choisir `esp32c6`, puis dans `.cargo/config.toml` remplacer la cible par `riscv32imafc-esp-espidf` et `MCU="esp32p4"`. Dans `[env]`, fixer `ESP_IDF_VERSION = "v5.5.1"`, ou la dernière version 5.5.x.

```bash
cd s1-p4-rust && cargo build --release
espflash flash --monitor target/riscv32imafc-esp-espidf/release/s1-p4-rust
```

Expected: `Hello, world!` sur la console. **Si ça échoue ici, S1 est no-go** : passer au Step 7 et décrire l'erreur.

- [ ] **Step 3: Wi-Fi AP + HTTP via `esp_hosted`**

Dans `Cargo.toml`, déclarer les composants C, aux versions de la v3 (`esp_wifi_remote` 0.14.*, `esp_hosted` ≥ 2.0.7) :

```toml
[[package.metadata.esp-idf-sys.extra_components]]
remote_component = { name = "espressif/esp_wifi_remote", version = "0.14.*" }
[[package.metadata.esp-idf-sys.extra_components]]
remote_component = { name = "espressif/esp_hosted", version = "*" }
```

Créer `sdkconfig.defaults` avec les options de la v3 :

```
CONFIG_FREERTOS_TIMER_TASK_STACK_DEPTH=2048
CONFIG_ESPTOOLPY_FLASHSIZE_16MB=y
CONFIG_ESP_HOSTED_RPC_TASK_STACK=4096
CONFIG_ESP_HOSTED_DFLT_TASK_STACK=3072
CONFIG_FREERTOS_IDLE_TASK_STACKSIZE=1536
```

Essayer d'abord l'API haut niveau :

```rust
let p = esp_idf_svc::hal::peripherals::Peripherals::take()?;
let sysloop = esp_idf_svc::eventloop::EspSystemEventLoop::take()?;
let nvs = esp_idf_svc::nvs::EspDefaultNvsPartition::take()?;
let mut wifi = esp_idf_svc::wifi::BlockingWifi::wrap(
    esp_idf_svc::wifi::EspWifi::new(p.modem, sysloop.clone(), Some(nvs))?, sysloop)?;
wifi.set_configuration(&esp_idf_svc::wifi::Configuration::AccessPoint(
    esp_idf_svc::wifi::AccessPointConfiguration {
        ssid: "S1-SPIKE".try_into().unwrap(), password: "spike-s1-pass".try_into().unwrap(),
        auth_method: esp_idf_svc::wifi::AuthMethod::WPA2Personal, channel: 6, ..Default::default() }))?;
wifi.start()?;
let mut http = esp_idf_svc::http::server::EspHttpServer::new(&Default::default())?;
http.fn_handler("/", esp_idf_svc::http::Method::Get, |req| {
    req.into_ok_response()?.write(b"S1 ok").map(|_| ())
})?;
```

Si `p.modem` n'existe pas pour le P4, ou si `EspWifi` ne compile pas, refaire le même enchaînement en appelant directement les fonctions C via `esp_idf_svc::sys` (`esp_netif_init`, `esp_wifi_init`, `esp_wifi_set_mode(WIFI_MODE_AP)`, `esp_wifi_set_config`, `esp_wifi_start`), comme `wifi_mgr_start_ap()` de la v3.

Expected: depuis un téléphone, `http://192.168.4.1/` affiche `S1 ok`. Noter laquelle des deux voies a fonctionné.

- [ ] **Step 4: 10 s d'I2S écrites en WAV sur la SD**

Mettre la SD sous tension comme la v3 : canal LDO 4 (`esp_ldo_acquire_channel`) à 3,3 V, puis GPIO45 en sortie à 0 (transistor P AO3401 actif bas). Monter ensuite la SD en SDMMC 4 bits (CLK=43, CMD=44, D0–D3=39–42), avec `esp_idf_svc::fs::fatfs` ou `esp_vfs_fat_sdmmc_mount` via `sys`.

Lire l'I2S0 en mode standard RX : 16 kHz, slot 32 bits, mono gauche, BCLK=21, WS=22, DIN=20. Écrire `/sdcard/s1.wav` avec un en-tête WAV PCM 16 bits mono de 44 octets, chaque échantillon étant `(mot32 >> 16) as i16`.

Expected: `s1.wav` s'ouvre sur PC et dure 10 s ±0,5 s, avec la voix audible.

- [ ] **Step 5: Réveil du deep sleep par GPIO5**

```rust
unsafe {
    esp_idf_svc::sys::esp_sleep_enable_timer_wakeup(60_000_000);
    esp_idf_svc::sys::esp_deep_sleep_enable_gpio_wakeup(
        (1 << 0) | (1 << 5),
        esp_idf_svc::sys::esp_deepsleep_gpio_wake_up_mode_t_ESP_GPIO_WAKEUP_GPIO_LOW,
    );
    esp_idf_svc::sys::esp_deep_sleep_start();
}
```

Au boot, afficher `esp_sleep_get_wakeup_cause()` et `esp_sleep_get_gpio_wakeup_status()`. Le GPIO5 doit être en pull-up : vérifier dans le TRM ESP32-P4 qu'il s'agit bien d'un GPIO LP et activer `rtc_gpio_pullup_en`. Pendant le sommeil, relier GPIO5 à GND avec le cavalier.

Expected: réveil immédiat, cause GPIO, bit 5 positionné. Sinon, R-05 s'applique : noter le repli prévu (C6 en parallèle du bouton BOOT, GPIO0).

- [ ] **Step 6: Mesures**

- Courant à l'entrée 5 V pendant 30 s de capture I2S, modem débranché : moyenne en mA.
- Courant en deep sleep.
- `ls -l` du binaire `.bin` produit par `espflash save-image` : il doit faire moins de 4 Mo pour tenir dans un slot OTA de spec §2.10.

- [ ] **Step 7: Écrire `docs/spikes/S1-p4-rust.md` et commiter**

```markdown
# Spike S1 — Rust std sur ESP32-P4 — <date>

| Critère (spec §3.3) | Résultat | Preuve (log, photo, mesure) |
|---|---|---|
| Hello Rust flashé | OK / KO | |
| Wi-Fi AP + HTTP (voie haut niveau / FFI) | | |
| 10 s I2S → WAV sur SD | | |
| Réveil deep sleep par GPIO5 | | |
| Courant capture audio / deep sleep (mA) | | |
| Taille du binaire (Mo) | | |

Versions qui fonctionnent : rustc +esp …, esp-idf-svc =…, esp-idf-hal =…, esp-idf-sys =…, ESP-IDF v5.5.…

**Décision : GO / NO-GO.** Si NO-GO : repli hybride (spec §3.3), main en C + `ruche-core` en staticlib.
```

```bash
git add spikes/s1-p4-rust docs/spikes/S1-p4-rust.md
git commit -m "spike(s1): Rust std sur ESP32-P4 - note go/no-go

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

Vérifier avec `git status` qu'aucun `target/` n'est ajouté.

---

### Task 10: Spike S2 — PPP `esp_modem` sur A7672E en USB (matériel requis)

Spike en **C** : on ne valide ici que `esp_modem` sur ce matériel. Son habillage en Rust viendra au plan 3.

**Files:**
- Create: `spikes/s2-ppp/` (jetable)
- Create: `docs/spikes/S2-ppp.md`

**Interfaces:**
- Produces: une note go / no-go, la configuration `esp_modem` qui fonctionne (DTE USB, interface, type d'appareil), et les durées mesurées (PPP up, publication MQTT).

- [ ] **Step 1: Créer le projet depuis l'exemple officiel**

```bash
mkdir -p spikes && cd spikes
. ~/esp/esp-idf/export.sh        # ESP-IDF v5.5
idf.py create-project-from-example "espressif/esp_modem:pppos_client"
mv pppos_client s2-ppp && cd s2-ppp
idf.py set-target esp32p4
idf.py menuconfig
```

Dans menuconfig :
- interface série = **USB** ;
- VID/PID = `0x1E0E` / `0x9011` ;
- type d'appareil = `A7670` s'il est proposé, sinon `SIM7600`, dont le jeu AT est proche ;
- APN = `internet` ;
- dans le sdkconfig, ajouter les options `CONFIG_ESP_HOSTED_*` et `FREERTOS_*` de la Task 9 uniquement si la compilation l'exige.

- [ ] **Step 2: Reprendre le démarrage du modem de la v3**

Au début de `app_main`, avant la création du DTE :

```c
gpio_reset_pin(GPIO_NUM_46);
gpio_set_direction(GPIO_NUM_46, GPIO_MODE_OUTPUT);
gpio_set_level(GPIO_NUM_46, 1);            /* alimentation modem (config.h v3) */
vTaskDelay(pdMS_TO_TICKS(18000));          /* MDM_BOOT_DELAY_MS v3 : boot + re-enumeration USB */
```

- [ ] **Step 3: Publier en mTLS sur le broker**

Remplacer le client MQTT de l'exemple par :

```c
extern const char ca_crt[]  asm("_binary_ca_crt_start");
extern const char cli_crt[] asm("_binary_client_crt_start");
extern const char cli_key[] asm("_binary_client_key_start");
esp_mqtt_client_config_t cfg = {
    .broker.address.uri = "mqtts://mqtt.example.com:8883",
    .broker.verification.certificate = ca_crt,
    .credentials.client_id = "cca-ruche-0001",
    .credentials.authentication.certificate = cli_crt,
    .credentials.authentication.key = cli_key,
};
```

Les certificats (jamais commités) se copient depuis ton dossier local de certificats mTLS vers `main/certs/` (`ca.crt`, `client.crt`, `client.key`), puis se déclarent dans `EMBED_TXTFILES`. **Ajouter `spikes/s2-ppp/main/certs/` au `.gitignore` avant tout commit.**

Une fois connecté, publier `{"spike":"s2","n":<i>}` sur `ruche/cca-ruche-0001/spike`, en QoS1.

- [ ] **Step 4: Boucler 20 cycles**

Un cycle :
1. PPP up (`esp_modem_set_mode(dce, ESP_MODEM_MODE_DATA)`) ;
2. attente de l'IP ;
3. MQTT connect + publication en QoS1 + attente du PUBACK ;
4. disconnect ;
5. retour en mode commande ;
6. attente de 60 s.

Le modem reste alimenté, comme `MDM_KEEP_POWERED=1`. Journaliser les durées de chaque phase.

Expected: 20 PUBACK sur 20, à vérifier avec un abonné sur `ruche/cca-ruche-0001/spike` côté broker.

- [ ] **Step 5: Note et commit**

`docs/spikes/S2-ppp.md` contient :
- un tableau : réussites sur 20, durée médiane de PPP up, durée médiane de publication, erreurs USB observées ;
- la configuration menuconfig retenue ;
- la décision **GO / NO-GO**. Si NO-GO : porter en Rust le tunnel AT + mbedTLS de la v3 (`modem.c`, copie privée ; spec §3.3).

```bash
git status --short spikes/s2-ppp | grep -i "certs" && echo "STOP: certificats suivis" || true
git add .gitignore spikes/s2-ppp docs/spikes/S2-ppp.md
git commit -m "spike(s2): PPP esp_modem sur A7672E USB - note go/no-go

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 11: Spike S3 — Zigbee coordinateur + BLE sur XIAO ESP32-C6 (matériel requis)

**Files:**
- Create: `spikes/s3-c6-coex/` (jetable)
- Create: `docs/spikes/S3-c6-coex.md`

**Interfaces:**
- Produces: une note go / no-go sur la coexistence 802.15.4 + BLE, la configuration d'antenne, et la vérification sur le schéma Seeed de l'isolation VBUS / broche 5V.

- [ ] **Step 1: Partir de l'exemple coordinateur de l'esp-zigbee-sdk**

```bash
mkdir -p spikes && cd spikes
git clone --depth 1 https://github.com/espressif/esp-zigbee-sdk.git ~/esp/esp-zigbee-sdk
cp -r ~/esp/esp-zigbee-sdk/examples/esp_zigbee_HA_sample/HA_on_off_switch s3-c6-coex
cd s3-c6-coex && idf.py set-target esp32c6
```

L'exemple `HA_on_off_switch` forme le réseau en coordinateur (ZC). Vérifier que `idf_component.yml` épingle `esp-zigbee-lib` et `esp-zboss-lib` en `~1.6.0`.

- [ ] **Step 2: Antenne du XIAO**

Au tout début de `app_main` :

```c
/* XIAO ESP32-C6 : GPIO3 = 0 active le commutateur RF, GPIO14 choisit l'antenne */
gpio_reset_pin(GPIO_NUM_3);
gpio_set_direction(GPIO_NUM_3, GPIO_MODE_OUTPUT);
gpio_set_level(GPIO_NUM_3, 0);
vTaskDelay(pdMS_TO_TICKS(100));
gpio_reset_pin(GPIO_NUM_14);
gpio_set_direction(GPIO_NUM_14, GPIO_MODE_OUTPUT);
gpio_set_level(GPIO_NUM_14, 1);   /* 1 = U.FL externe, 0 = ceramique */
```

- [ ] **Step 3: Ajouter un périphérique NimBLE avec passkey**

Copier `gatt_svr.c` et la partie GAP de `$IDF_PATH/examples/bluetooth/nimble/bleprph/main/main.c` dans `main/`.

Dans `sdkconfig.defaults`, ajouter :

```
CONFIG_BT_ENABLED=y
CONFIG_BT_NIMBLE_ENABLED=y
CONFIG_ESP_COEX_SW_COEXIST_ENABLE=y
CONFIG_BT_NIMBLE_SM_SC=y
```

Configurer la sécurité :

```c
ble_hs_cfg.sm_io_cap = BLE_SM_IO_CAP_DISP_ONLY;   /* passkey affiche cote ruche */
ble_hs_cfg.sm_mitm = 1;
ble_hs_cfg.sm_sc = 1;
ble_hs_cfg.sm_bonding = 1;
```

Sur `BLE_GAP_EVENT_PASSKEY_ACTION` avec `BLE_SM_IOACT_DISP`, répondre la passkey fixe `123456` par `ble_sm_inject_io()`. La caractéristique d'écriture porte les drapeaux `BLE_GATT_CHR_F_WRITE | BLE_GATT_CHR_F_WRITE_ENC`. À l'écriture de `OPEN_PORTAL`, faire une impulsion basse de 50 ms sur D2 (GPIO2), en sortie open-drain.

- [ ] **Step 4: Essai de coexistence pendant 1 h**

1. Appairer 5 appareils Zigbee : réels (T/H, contact), ou un second C6 avec l'exemple `HA_temperature_sensor`.
2. Les configurer pour envoyer un report toutes les 60 s.
3. Pendant 1 h, se connecter en BLE avec nRF Connect toutes les 2 min : appairage, écriture de `OPEN_PORTAL`, déconnexion.
4. Compter les reports reçus par appareil dans le log.
5. Vérifier l'impulsion sur D2 à l'oscilloscope ou avec une LED.

Expected: au moins 59 reports par appareil, aucune déconnexion Zigbee, et l'appairage BLE refusé sans le bon passkey.

- [ ] **Step 5: Vérifier l'alimentation du XIAO**

Sur le schéma officiel Seeed du XIAO ESP32-C6, regarder si une diode ou un switch isole VBUS (USB-C) de la broche 5V. Noter le résultat et la procédure de flash sur place qui en découle : débrancher le 5 V externe, ou non.

- [ ] **Step 6: Note et commit**

`docs/spikes/S3-c6-coex.md` contient :
- un tableau des reports reçus par appareil (sur 60 attendus) et des connexions BLE réussies ou refusées ;
- l'antenne utilisée ;
- le résultat de la vérification VBUS ;
- les versions `esp-zigbee-lib` et `esp-zboss-lib` ;
- la décision **GO / NO-GO**. Si NO-GO : BLE retiré du C6 (spec §3.3).

```bash
git add spikes/s3-c6-coex docs/spikes/S3-c6-coex.md
git commit -m "spike(s3): coexistence Zigbee ZC + NimBLE sur XIAO ESP32-C6 - note go/no-go

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Après ce plan

Une fois les trois notes de spike écrites :

1. Reporter dans la spec (§3.2) les versions validées, ainsi que l'écart de protocole noté à la Task 1 (`zone_type`, `analog_units`).
2. Écrire le plan 2 (firmware C6), en reprenant le protocole UART testé dans `ruche-core::c6_proto`, côté C cette fois.
3. Écrire le plan 3 (firmware P4), qui branche les drivers validés au S1 et au S2 sur `ruche-core`.
4. Écrire le plan 4 (portail, OTA, release).

## Self-Review

- **Couverture :**
  - L1 : `config` (Task 4), `names` (Task 3), `zcl` (Task 2), `c6_proto` (Task 1), `payload` (Task 8), et `bridge` (Task 5), ajouté pour FR-ZB-04/05/06 et §2.3.2.
  - L2 : `audio` (Task 6), `alarm` (Task 7).
  - Spikes : S1 (Task 9), S2 (Task 10), S3 (Task 11).
  - L3 à L8 : renvoyés explicitement aux plans 2 à 4.
- **Tests de spec couverts :** T-UT-01 (zcl), T-UT-02 (c6_proto), T-UT-03 (names), T-UT-04 (audio), T-UT-05 (alarm), T-UT-06 (payload, bridge), T-UT-07 (config). T-UT-08 relève du firmware C6 (plan 2).
- **Cohérence des types :** `Ieee`, `DeviceInfo`, `Report` et `Event` (Task 1) sont réutilisés tels quels dans les Tasks 3, 4, 5 et 8. `Settings` (Task 4) sert aux Tasks 6 et 7. `WindowSummary` et `Rule` (Task 6) et `AlarmEvent` (Task 7) servent à la Task 8.
- **Code vérifié :** les modules ont été compilés et testés tels quels (38 tests, `clippy -D warnings` et `fmt --check` propres, rustc 1.94.1) avant d'être intégrés à ce plan.
