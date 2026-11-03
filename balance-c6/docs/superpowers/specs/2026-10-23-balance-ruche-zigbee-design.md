# Balance de ruche Zigbee — XIAO ESP32-C6 + NAU7802

| Champ | Valeur |
|---|---|
| Version | 1.0 — 2026-10-23 |
| Statut | Spec validée en conversation, à relire |
| Cible | Firmware Arduino (`.ino`) pour Seeed Studio XIAO ESP32-C6 |
| Contrat amont | Spec RUCHE CCA V27.01 §2.3.2 (`concentrateur-p4/docs/superpowers/specs/2026-10-21-ruche-cca-v27-01-design.md`) |
| Banc de test | Zigbee2MQTT + dongle, au laboratoire |

## 1. Objectif

Un nœud de pesée sur batterie qui rejoint le réseau Zigbee 3.0 du concentrateur V27, publie le poids de la ruche et sa batterie, et se règle à distance depuis le coordinateur (tare, étalonnage, intervalle, offset/facteur, nom), à la manière d'un appareil Sonoff, Shelly ou Aqara.

Critères de réussite :
- Le concentrateur V27 reconnaît le nœud sans code spécifique (contrat §2.3.2 respecté à l'identique).
- Avec Zigbee2MQTT au banc, chaque réglage s'écrit, s'applique et survit à une coupure d'alimentation.
- Autonomie de 8 mois ou plus avec une LiPo de 1000 mAh à l'intervalle par défaut (3600 s).

Hors périmètre :
- Côté concentrateur V27 : la file d'écritures descendantes et la lecture du nom. Voir §9, c'est un chantier séparé.
- OTA Zigbee, mode routeur, cluster fabricant, compensation en température.

## 2. Matériel

| Élément | Référence |
|---|---|
| Microcontrôleur | Seeed Studio XIAO ESP32-C6 (antenne intégrée, déjà sélectionnée par le variant Arduino : GPIO3 = 0, GPIO14 = 0) |
| Convertisseur | Adafruit NAU7802 (24 bits, I2C 0x2A, STEMMA QT) |
| Capteur | Variohm AL6N-C3-100kg, 2,0 mV/V, pont de 350 Ω, câble 6 fils |
| Batterie | LiPo 1S sur les pads BAT du XIAO (charge par USB-C) |

### 2.1 Câblage

| Liaison | Détail |
|---|---|
| NAU7802 SDA / SCL | D4 (GPIO22) / D5 (GPIO23) ; pull-ups présents sur la carte Adafruit |
| NAU7802 VIN / GND | 3V3 / GND du XIAO |
| AL6N → NAU7802 | E+ et Sense+ → E+ ; E− et Sense− → E− ; S+ → A+ ; S− → A− (couleurs à lire sur l'étiquette du câble) ; blindage à GND côté carte uniquement |
| Mesure batterie | Pont 200 kΩ / 200 kΩ entre BAT+ et GND, point milieu sur A0 (GPIO0), 100 nF entre A0 et GND |
| LED « ON » de la carte NAU7802 | Si elle existe, couper son cavalier : elle consommerait en permanence |

### 2.2 Réglages NAU7802

- LDO interne à **3,0 V**, pour garder une marge sous le 3,3 V du XIAO.
- Gain **×128**, **10 SPS**.
- Au réveil : attente de 600 ms (datasheet), puis calibration **interne uniquement** (`CALMOD_INTERNAL`). La calibration offset système est interdite, car elle effacerait la charge posée sur le plateau.
- Entre deux mesures : power-down (PUA = PUD = 0, moins de 1 µA). Le LDO est alors coupé, donc le pont n'est plus alimenté, et les registres sont conservés.
- Ordre de grandeur : environ 21 475 points/kg, soit une résolution très inférieure au gramme.

## 3. Modèle Zigbee

- Type : End Device (`ZIGBEE_MODE_ED`), dormant par deep sleep. Pendant l'éveil, la radio reste en réception (`rx_on_when_idle = true`, valeur par défaut de la librairie, comme dans l'exemple sleepy d'Espressif). Les écritures du coordinateur arrivent donc immédiatement pendant la fenêtre d'écoute, sans attendre une interrogation du parent.
- Cluster Basic : fabricant `CCA`, modèle `CCA-RUCHE-WEIGHT`, source d'alimentation = batterie.

| EP | Cluster serveur | Description (`0x001C`) | Accès | Rôle |
|---|---|---|---|---|
| 1 | Analog Input `0x000C` | `weight` | R, report | `0x0055` PresentValue (float, kg), `0x0075` EngineeringUnits = 39 |
| 1 | Power Config `0x0001` | — | R, report | `0x0020` tension (unité de 100 mV), `0x0021` batterie % (unité de 0,5 %) |
| 1 | Basic `0x0000` | — | RW | `0x0010` LocationDescription = nom de la balance (§3.1) |
| 2 | Analog Output `0x000D` | `tare` | W | Toute écriture déclenche la tare |
| 3 | Analog Output | `calibration_kg` | W | Masse connue posée (> 0) → calcul du facteur |
| 4 | Analog Output | `interval_s` | RW | Période de réveil, de 60 à 86 400 s, 3600 par défaut |
| 5 | Analog Output | `offset` | RW | Zéro brut en points ADC, 0 par défaut |
| 6 | Analog Output | `scale` | RW | Points ADC par kg, 21 475 par défaut |

L'attribut EngineeringUnits n'est pas créé par `ZigbeeAnalog` (core 3.3.8). Une sous-classe du sketch l'ajoute au cluster Analog Input après `addAnalogInput()`. Cette même sous-classe surcharge `zbAttributeSet` pour intercepter les écritures sur le Basic `0x0010`.

Les endpoints 2 à 6 portent des Analog Output, jamais des Analog Input. Le concentrateur V27 ne les confond donc pas avec la mesure du contrat.

Pourquoi des Analog Output standards plutôt qu'un cluster fabricant : la spec V27 exclut les clusters fabricant, et ZHA comme Zigbee2MQTT savent lire et écrire ces clusters standards.

### 3.1 Nom de la balance

- Basic `0x0010` LocationDescription : de 1 à 16 caractères ASCII imprimables (0x20 à 0x7E).
- Valeur par défaut : `ruche-XXXX`, où XXXX sont les 4 derniers caractères hexadécimaux de l'adresse IEEE.
- Le nom est persisté en NVS et republié dans l'attribut à chaque démarrage.
- Une écriture vide, trop longue ou contenant des caractères non imprimables est ignorée, et l'ancien nom est rétabli dans l'attribut.

### 3.2 Calculs

- `poids_kg = (brut − offset) / scale`
- Tare : `offset = brut` (nouvelle pesée).
- Étalonnage avec la masse `m` : `scale = (brut − offset) / m` (nouvelle pesée). Refusé si `m ≤ 0` ou si `|scale| < 1000`.
- Écriture directe de `scale` : refusée si `|scale| < 1000` (ce qui couvre 0 et la division par zéro).
- `interval_s` : ramené dans la plage [60, 86 400].
- Batterie : `V_bat = 2 × V_A0`, avec une moyenne de 16 lectures `analogReadMilliVolts`. Le % est interpolé linéairement par morceaux : 3,30 V = 0 %, 3,60 V = 10 %, 3,70 V = 30 %, 3,80 V = 60 %, 3,95 V = 85 %, 4,20 V = 100 %.

Toute écriture refusée laisse l'ancienne valeur en place et republie cette valeur dans l'attribut concerné.

### 3.3 Persistance

`Preferences`, namespace `balance` : `offset` (float), `scale` (float), `interval` (u32), `name` (string). Chaque écriture acceptée est sauvegardée immédiatement.

## 4. Cycle de fonctionnement

### 4.1 Réveil par timer (cycle normal)

1. Réveil du NAU7802, attente de 600 ms, calibration interne.
2. 4 échantillons jetés, puis moyenne de 10 (environ 1,5 s), puis power-down du NAU7802.
3. Mesure de la batterie.
4. `Zigbee.begin()` en End Device, avec un timeout de 10 s et une reprise du réseau depuis la flash.
5. Report du poids (EP1) et de la batterie %, avec attente de l'acquittement (3 essais, délai de 1 s chacun).
6. **Fenêtre d'écoute de 5 s** pour les écritures du coordinateur (§4.3).
7. Deep sleep de `interval_s − durée d'éveil`, avec un minimum de 10 s.

### 4.2 Démarrage hors timer : mode réglage

Une mise sous tension ou un appui sur RESET (cause de réveil ≠ timer) fait entrer en mode réglage :
- Le nœud reste éveillé **180 s** et refait une pesée avec report toutes les **10 s**.
- Ce mode sert au premier appairage, ainsi qu'à la tare et à l'étalonnage au banc.
- À la fin des 180 s, le nœud passe en deep sleep et revient au cycle normal.
- **Appui de 10 s sur BOOT (GPIO9)** : remise à zéro usine Zigbee (`Zigbee.factoryReset()`), qui quitte le réseau puis redémarre. Les réglages en NVS (`balance`) sont conservés.

BOOT ne peut pas réveiller le C6 depuis le deep sleep (seuls GPIO0 à 7 le peuvent) : on le surveille uniquement quand le nœud est éveillé.

### 4.3 Traitement des écritures

Le callback Zigbee se contente de copier la valeur et de lever un drapeau. La boucle principale fait ensuite le travail, en dehors du contexte de la pile Zigbee.

| Écriture | Action |
|---|---|
| `tare` | Nouvelle pesée, `offset = brut`, sauvegarde, mise à jour de l'EP5, report du poids |
| `calibration_kg = m` | Nouvelle pesée, calcul du facteur (§3.2), sauvegarde, mise à jour de l'EP6, report du poids |
| `interval_s` | Bornage, sauvegarde, application au prochain sommeil |
| `offset` | Sauvegarde, report du poids recalculé avec le dernier brut |
| `scale` | Validation, sauvegarde, report du poids recalculé avec le dernier brut |
| nom (`0x0010`) | Validation, sauvegarde |

Une écriture reçue pendant la fenêtre de 5 s prolonge celle-ci de 5 s, pour permettre d'enchaîner plusieurs réglages.

## 5. Gestion des erreurs

| Cas | Comportement |
|---|---|
| NAU7802 absent ou sans donnée prête (délai de 2 s) | Pas de report de poids, log série ; la batterie est quand même reportée |
| `Zigbee.begin()` en échec ou réseau introuvable | Deep sleep de `interval_s`, puis nouvel essai au cycle suivant. Jamais de redémarrage en boucle. |
| Report non acquitté après 3 essais | Log, puis fenêtre d'écoute et sommeil normaux |
| Valeur écrite invalide | Refus (§3.2, §3.1), ancienne valeur republiée |

## 6. Autonomie (estimation)

- Éveil d'environ 4 s à ~80 mA, plus un sommeil à ~25 µA (XIAO, pont batterie ~9 µA, NAU7802 < 1 µA).
- À 3600 s : environ 0,15 mAh/h, soit **8 à 9 mois** sur 1000 mAh.
- À 300 s : environ 1 mois.
- Le mode réglage (180 s) coûte environ 4 mAh par utilisation.

## 7. Organisation du code

```
BalanceRuche/
  BalanceRuche.ino   setup/loop, cycle, deep sleep, mode réglage, bouton BOOT
  zb_weight.h        sous-classe ZigbeeAnalog (EngineeringUnits, nom 0x0010), 6 EP, drapeaux d'écriture
  scale_math.h       fonctions pures : poids, tare, étalonnage, bornage, % batterie, validation du nom
  config.h           broches et constantes
tests/
  test_scale_math.cpp   asserts, compilés avec g++ sur PC
z2m/
  cca_ruche_weight.mjs  external converter Zigbee2MQTT pour le banc
```

`scale_math.h` ne dépend pas d'Arduino, ce qui permet de le compiler sur PC.

### 7.1 Outillage

- IDE Arduino avec le core **esp32 3.3.8**.
- Carte **XIAO_ESP32C6**.
- *Zigbee mode* : **Zigbee ED (end device)**.
- *Partition scheme* : **Zigbee 4MB with spiffs**.
- Librairie : **Adafruit NAU7802** (avec ses dépendances, dont Adafruit BusIO, déjà installée).

## 8. Tests

1. **Unitaires (PC)** : `g++ -std=c++17 tests/test_scale_math.cpp -o /tmp/t && /tmp/t`. Couvre la tare, l'étalonnage (y compris les refus), le bornage de l'intervalle, le refus d'un facteur trop faible, le % batterie (bornes et interpolation) et la validation du nom.
2. **Compilation** : `arduino-cli compile` pour `esp32:esp32:XIAO_ESP32C6` avec `ZigbeeMode=ed` et la partition Zigbee, si `arduino-cli` est disponible ; sinon dans l'IDE.
3. **Recette au banc (Zigbee2MQTT)** :
   1. Appairage en mode réglage : le modèle `CCA-RUCHE-WEIGHT` est reconnu avec le converter.
   2. Le poids et la batterie remontent.
   3. Tare à vide : le poids vaut 0 ± 0,05 kg.
   4. Étalonnage avec une masse connue : l'erreur est inférieure ou égale à 0,1 kg sur une seconde masse.
   5. Écriture de `interval_s` : la période observée change.
   6. Écriture du nom : il se relit après une coupure d'alimentation.
   7. Après coupure d'alimentation : offset, scale, intervalle et nom sont conservés, et le nœud se reconnecte sans réappairage.
   8. Appui de 10 s sur BOOT : le nœud quitte le réseau et peut être réappairé.
   9. Écriture invalide (`scale = 0`, `calibration_kg = -1`, nom vide) : refusée, et l'ancienne valeur est relue.

## 9. Impacts sur le concentrateur V27 (chantier séparé)

À reporter dans la spec V27 ; rien ne bloque ce firmware.

1. **File d'écritures (`toZigbee`)** : une commande UART `write` mise en file dans le C6. Elle est envoyée dès que le nœud se manifeste (report reçu), pendant sa fenêtre d'écoute de 5 s. Comme le nœud annonce `rx_on_when_idle = true`, le parent ne bufferise pas pour lui : une écriture envoyée pendant son sommeil est perdue, d'où la nécessité de cette file.
2. **Interview** : lire aussi le Basic `0x0010` pour proposer le nom comme `friendly_name` par défaut.
3. **Reporting** : la configuration envoyée à l'interview (min 300 / max 3600 / Δ 0,1 kg) n'a pas d'effet sur un nœud en deep sleep. Il reporte à chaque réveil, selon `interval_s`. Garder `interval_s ≤ weight_max_age_s` (7200 s par défaut), sinon `weight_kg` vaudra `null`.
4. **Power Config `0x0020`** : la librairie Arduino déclare la tension batterie comme non reportable. Le C6 doit tolérer l'échec du Configure Reporting sur cet attribut et le lire à la demande.
