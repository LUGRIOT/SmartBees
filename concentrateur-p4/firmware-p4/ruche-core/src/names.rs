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
