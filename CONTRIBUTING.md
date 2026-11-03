# Contribuer à SmartBees

Merci de votre intérêt. Le projet comporte deux firmwares qui travaillent ensemble :

| Dossier | Quoi | Langage |
|---|---|---|
| `concentrateur-p4/` | Concentrateur de rucher V27 sur ESP32-P4-NANO, avec un coordinateur Zigbee XIAO ESP32-C6 | Rust (`firmware-p4/ruche-core`), C pour le coordinateur |
| `balance-c6/` | Nœud de pesée Zigbee, XIAO ESP32-C6 + NAU7802 | Arduino (C++) |

Avant de modifier un comportement, lisez la spec concernée. Elle fait autorité, et une pull request qui s'en écarte doit la mettre à jour dans le même changement :
- `concentrateur-p4/docs/superpowers/specs/2026-10-21-ruche-cca-v27-01-design.md`
- `balance-c6/docs/superpowers/specs/2026-10-23-balance-ruche-zigbee-design.md`

## Proposer un changement

1. Ouvrez d'abord une issue pour tout changement de comportement, de protocole ou de matériel. Une correction évidente peut partir directement en pull request.
2. Forkez le dépôt, puis créez une branche depuis `main` : `feat/…`, `fix/…` ou `docs/…`.
3. Faites de petits commits au format [Conventional Commits](https://www.conventionalcommits.org/fr/) : `feat(core): …`, `fix(balance): …`, `docs(spec): …`.
4. Ouvrez une pull request vers `main`. La CI doit être verte, et une relecture est nécessaire avant la fusion.

## Tester avant d'envoyer

Ces tests tournent sur PC, sans matériel. Ce sont les mêmes que ceux de la CI.

```bash
# Concentrateur : logique metier
cd concentrateur-p4/firmware-p4/ruche-core
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test

# Balance : calculs et controles du sketch
cd balance-c6
mkdir -p build
g++ -std=c++17 -Wall -Wextra tests/test_scale_math.cpp -o build/test_scale_math && build/test_scale_math
sh tests/check_sketch.sh
```

La CI utilise Rust 1.94.1. Si votre version est plus récente et que clippy signale des lints nouveaux, corrigez-les ou signalez-le dans la PR.

Une nouvelle logique arrive avec son test : écrivez le test d'abord, vérifiez qu'il échoue, puis implémentez. Ce qui touche au matériel (radio, I2S, modem, consommation) se valide au banc : décrivez dans la PR ce que vous avez mesuré, et sur quel matériel.

## Règles du code

- **`ruche-core` reste sans dépendance ESP.** Tout ce qui s'y trouve doit se tester sur PC. N'ajoutez pas de dépendance sans expliquer pourquoi dans la PR.
- Commentaires en français, qui expliquent le *pourquoi* plutôt que le *quoi*. Une temporisation ou un seuil est accompagné de sa mesure ou de sa source.
- Toute donnée venant d'un appareil ou du réseau est considérée comme hostile : elle est bornée et validée en entrée, et échappée en sortie (JSON, page web, topics MQTT).

## Ajouter un appareil Zigbee

Un capteur qui respecte les clusters ZCL standard fonctionne sans code. Pour un autre (spec §6.4) :

1. Capturez ses reports au banc dans `concentrateur-p4/firmware-p4/ruche-core/tests/captures/<modele>.jsonl`.
2. Comparez avec le converter du même modèle dans [zigbee-herdsman-converters](https://github.com/Koenkk/zigbee-herdsman-converters) : les noms de propriétés suivent ceux de Zigbee2MQTT.
3. Ajoutez l'entrée dans `zcl.rs`, avec un test qui rejoue la capture.

## Secrets : jamais dans le dépôt

Ne commitez jamais de certificat, de clé privée, de mot de passe Wi-Fi, de passkey BLE ou d'identifiant de SIM. Le `.gitignore` bloque déjà `*.key`, `*.crt` et `*.pem`. Les secrets de chaque ruche sont provisionnés hors du dépôt (spec V27 §5.1).

Si un secret a été poussé par erreur, le supprimer dans un nouveau commit ne suffit pas : il reste dans l'historique. Prévenez immédiatement les mainteneurs dans une issue, sans y recopier le secret, pour qu'il soit révoqué.

## Licence

En contribuant, vous acceptez que votre contribution soit publiée sous la licence MIT du projet (voir `LICENSE`).
