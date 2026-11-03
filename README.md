# SmartBees

Ruche connectée : un concentrateur de rucher sur ESP32-P4 et une balance de ruche Zigbee sur XIAO ESP32-C6.

| Dossier | Contenu |
|---|---|
| [`concentrateur-p4/`](concentrateur-p4/) | Concentrateur V27 (ESP32-P4-NANO) : pont Zigbee vers MQTT inspiré de Zigbee2MQTT, écoute acoustique INMP441 et alarme, carte SD, LTE A7672E, portail local activé par BLE. Logique métier en Rust (`firmware-p4/ruche-core`). Le firmware v3 en C n'est pas publié : la V27 le remplace. |
| [`balance-c6/`](balance-c6/) | Nœud de pesée Zigbee (XIAO ESP32-C6 + NAU7802 + cellule Variohm AL6N 100 kg), firmware Arduino, avec un converter Zigbee2MQTT pour le banc. |

## Documents

- Spec du concentrateur, cycle Requirements → Maintain : [`concentrateur-p4/docs/superpowers/specs/2026-10-21-ruche-cca-v27-01-design.md`](concentrateur-p4/docs/superpowers/specs/2026-10-21-ruche-cca-v27-01-design.md)
- Architecture d'ensemble, à ouvrir dans un navigateur : [`concentrateur-p4/docs/architecture/ruche-cca-v27-architecture.html`](concentrateur-p4/docs/architecture/ruche-cca-v27-architecture.html)
- Spec de la balance : [`balance-c6/docs/superpowers/specs/2026-10-23-balance-ruche-zigbee-design.md`](balance-c6/docs/superpowers/specs/2026-10-23-balance-ruche-zigbee-design.md)

## Tester

```bash
# logique du concentrateur (sur PC, sans matériel)
cd concentrateur-p4/firmware-p4/ruche-core && cargo test

# calculs de la balance (sur PC)
g++ -std=c++17 balance-c6/tests/test_scale_math.cpp -o /tmp/t && /tmp/t
```

## Secrets

Aucun certificat, clé ni mot de passe n'est publié. Les certificats mTLS et les secrets de chaque ruche sont provisionnés hors du dépôt (spec §5.1).

## Licence

MIT, voir [`LICENSE`](LICENSE).
