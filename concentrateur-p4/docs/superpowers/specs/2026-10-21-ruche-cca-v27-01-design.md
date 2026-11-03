# RUCHE CCA V27.01 — Concentrateur Zigbee, acoustique et alarme cloud

| Champ | Valeur |
|---|---|
| Version du document | 1.2 — 2026-10-26 (nœud de pesée aligné sur le projet `ESP32-C6-BalanceRuche`, réglages à distance) |
| Branche | `RUCHE-CCA-V27.01` (git refuse les espaces dans un nom de branche) |
| Version firmware visée | `27.01.x` (P4 et C6 versionnés ensemble) |
| Statut | Spec à relire, puis plan d'implémentation |
| Base existante | `cca-ruche/` (v3, C / ESP-IDF v5.5), non publiée : voir la note ci-dessous |
| Remplace | Le prototype P4 + C6 du 2026-10-11 (protocole UART v1 limité à `devices` + `permit_join`), non publié |

Le firmware v3 (`cca-ruche/`, C / ESP-IDF v5.5) n'est **pas publié** dans ce dépôt : il contient des failles connues (XSS du tableau de bord, API sans authentification, mesh UDP non authentifié) et la V27 le remplace. Les mentions de ses fichiers renvoient à la copie privée du mainteneur ; les valeurs utiles (délais du modem, broches SD, sdkconfig) sont reprises ici.

Modèle de cycle de vie : **Requirements → Design → Build → Test → Deploy → Maintain**. Chaque exigence porte un identifiant repris en Design, en Test (§4.6, matrice de traçabilité) et en Maintain.

Décisions validées le 2026-10-21 :

1. Interface locale : **le BLE active l'AP Wi-Fi**, et la page de gestion est servie en HTTP par le P4.
2. Écoute audio : **par fenêtres périodiques**, pas en continu.
3. Langages : **firmware P4 en Rust** ; **firmware C6 coordinateur en C**, car le SDK Zigbee d'Espressif (ZBOSS) n'existe qu'en C.
4. Architecture Zigbee : **option A**. Le C6 sert de coordinateur autonome, sur le modèle de « herdsman ». Le P4 sert de pont façon « zigbee2mqtt ».

---

## 1. REQUIREMENTS

### 1.1 Objectif

L'ESP32-P4-NANO installé au rucher devient le **concentrateur** du rucher. Il collecte les données des capteurs Zigbee environnants, écoute la ruche avec un micro INMP441, enregistre des sons sur la carte SD et publie le tout sur le broker MQTT cloud (SmartBees) par LTE. Si l'acoustique sort de la consigne, il déclenche une alarme. L'installation et la configuration se font sur place, avec un smartphone.

### 1.2 Périmètre

**Dans le périmètre**
- Firmware P4 en Rust : pont Zigbee→MQTT, audio, SD, LTE, portail local, gestion de l'énergie.
- Firmware du C6 externe (**Seeed Studio XIAO ESP32-C6**) en C : coordinateur Zigbee, tampon des reports, BLE d'activation, ligne de réveil du P4.
- Protocole UART P4↔C6 v2.
- Topics et payloads MQTT V27.
- Page web de gestion embarquée.

**Hors périmètre**
- **Firmware du nœud de pesée** : projet séparé `ESP32-C6-BalanceRuche`, avec sa propre spec du 2026-10-23. C'est un firmware Arduino pour XIAO ESP32-C6, avec un Adafruit NAU7802 et une cellule Variohm AL6N-C3-100kg. Le HX711 est retiré du P4. Cette spec-ci fixe le **contrat Zigbee** du nœud (§2.3.2), et le concentrateur fournit ses **réglages à distance** (FR-ZB-09). Sa mesure alimente le champ `weight_kg` de `data`.
- Plateforme cloud : règles d'alarme côté serveur, tableaux de bord, notifications.
- Envoi des enregistrements audio vers le cloud, à cause du coût LTE.
- OTA par LTE.
- Commandes descendantes cloud→ruche : le P4 dort entre ses cycles. Les réglages du nœud de pesée envoyés depuis le **portail local** sont, eux, dans le périmètre (FR-ZB-09).
- Clusters fabricant (Tuya `0xEF00`, etc.), OTA Zigbee, mode routeur.
- **Mesh UDP inter-ruches** de la v3 : supprimé, Zigbee le remplace.
- PCB, certification radio.

### 1.3 Exigences fonctionnelles

**Zigbee (coordinateur et pont)**

| ID | Exigence | Critère d'acceptation |
|---|---|---|
| FR-ZB-01 | Le C6 forme un réseau Zigbee 3.0 comme coordinateur et le conserve en NVS après un redémarrage. | Après une coupure du C6, les appareils déjà appairés envoient de nouveau leurs reports sans être réappairés. |
| FR-ZB-02 | L'inclusion (permit join) ne s'ouvre que sur commande, pour 1 à 254 s, et reste fermée au boot. | `permit_join 0` et `255` sont refusés ; après un reboot, le réseau est fermé. |
| FR-ZB-03 | À l'arrivée d'un appareil, le C6 l'« interviewe » : cluster Basic (fabricant, modèle, source d'alimentation), endpoints, simple descriptors. Il établit ensuite le bind et configure les reports des clusters pris en charge (§2.4). | Un capteur T/H du commerce envoie son premier report en moins de 2 min après l'appairage. |
| FR-ZB-04 | Pendant que le P4 dort, le C6 met en tampon les reports d'attributs (anneau d'au moins 512 entrées, numérotées), puis le P4 les récupère au réveil. | Aucune perte sur un cycle de 5 min avec 32 appareils qui envoient chacun 1 report par minute. |
| FR-ZB-05 | Le P4 convertit les attributs ZCL en propriétés JSON nommées, à l'aide de tables de converters inspirées de Zigbee2MQTT (§2.2, §2.5.3). | Sortie identique à celle de Zigbee2MQTT, à l'arrondi près, pour les appareils de référence (§4.3, T-INT-04). |
| FR-ZB-06 | Chaque appareil a un nom lisible modifiable (friendly name) ; à défaut, son adresse IEEE. | Le renommage depuis le portail change le topic au cycle suivant. |
| FR-ZB-07 | Le portail permet d'exclure un appareil, de lire le canal, le PAN ID et le PAN ID étendu, et de réinitialiser le réseau avec un nouveau canal ou PAN (confirmation obligatoire). | Après réinitialisation, la liste d'appareils est vide et le nouveau canal est actif. |
| FR-ZB-08 | Capacité : 32 appareils finaux directs (end devices). | Banc d'essai : 32 appareils simulés (§4.3). |
| FR-ZB-09 | Depuis le portail, le nœud de pesée se règle à distance : tare, étalonnage avec une masse connue, intervalle de réveil, offset, facteur d'échelle et nom. Les écritures attendent dans une file du C6 et sont délivrées au réveil suivant du nœud (§2.3.2, §2.4). | Une tare demandée pendant le sommeil du nœud est appliquée à son réveil suivant, confirmée par `write_done`. La pesée suivante, plateau vide, vaut 0 ± 0,05 kg. |

**Télémétrie MQTT**

| ID | Exigence | Critère |
|---|---|---|
| FR-MQ-01 | Publication par LTE, en mTLS, vers le broker cloud. | Connexion acceptée par le broker de production (port 8883, nom fourni au provisioning). |
| FR-MQ-02 | Le topic `ruche/<HIVE_ID>/data` reste compatible v3 : mêmes champs, `weight_kg` compris, qui vient désormais du nœud de pesée Zigbee (§2.3.2). Seul `satellites` est retiré. | Le consommateur cloud v3 continue d'ingérer les données. `weight_kg` vaut `null` si le nœud de pesée est absent ou si sa mesure est périmée. |
| FR-MQ-03 | Un topic par appareil Zigbee, plus des topics `bridge/*`, sur le modèle de Zigbee2MQTT (§2.6). | Topics conformes au schéma §2.6. |
| FR-MQ-04 | Hors ligne, les messages vont dans la file SD et sont repris dans l'ordre (FIFO). Une alarme passe toujours en tête de file. | Après 24 h sans réseau, tout est publié au retour du réseau, alarmes d'abord. |
| FR-MQ-05 | Intervalle de cycle réglable : 1, 5, 15, 30 min, 1, 2, 4, 6, 12 ou 24 h. 5 min par défaut. | Mêmes valeurs que `ALLOWED_INTERVALS` en v3. |

**Audio**

| ID | Exigence | Critère |
|---|---|---|
| FR-AU-01 | Le P4 capte l'INMP441 en I2S, à 16 kHz en mono, pendant une fenêtre réglable de 5 à 120 s (30 s par défaut) à chaque cycle. | Durée de la fenêtre mesurée à ±1 s près. |
| FR-AU-02 | Chaque trame de 256 ms fournit : le niveau en dB SPL (étalonnable), la fréquence dominante entre 50 et 2000 Hz, et l'énergie de 4 bandes (100–200, 200–300, 300–500 et 500–1000 Hz). | Avec un son pur de 250 Hz, la fréquence dominante est mesurée à ±8 Hz. |
| FR-AU-03 | La consigne, réglable, comprend : niveau min/max, fréquence dominante min/max, et rapport maximal `bande 300–500 / bande 200–300`. | Valeurs persistées en NVS. |
| FR-AU-04 | Enregistrement WAV (PCM 16 bits) sur la SD, selon le mode `off`, `on_alarm` (par défaut) ou `always`. | Le fichier se lit sur un PC ; son nom porte l'horodatage et le motif. |
| FR-AU-05 | Rotation : les plus anciens enregistrements sont supprimés quand l'espace libre passe sous 10 % ou que le quota (1024 Mo par défaut) est dépassé. | La SD n'est jamais pleine à cause de l'audio. |

**Alarme**

| ID | Exigence | Critère |
|---|---|---|
| FR-AL-01 | Une fenêtre est « hors consigne » si au moins `breach_ratio` (60 % par défaut) de ses trames enfreignent une règle. | Test unitaire avec des trames synthétiques. |
| FR-AL-02 | L'alarme est levée après `confirm_windows` (2 par défaut) fenêtres hors consigne consécutives. Elle retombe après `clear_windows` (2 par défaut) fenêtres conformes. | Test unitaire de la machine d'états. |
| FR-AL-03 | Les messages `raised` et `cleared` sont publiés en QoS1 sur `ruche/<HIVE_ID>/alarm`, avec les métriques et le nom du fichier WAV. Ils partent dans le cycle même où l'alarme change d'état. | Au banc, l'alarme est reçue en moins de 2 cycles après le début d'un son à 450 Hz. |
| FR-AL-04 | Délai de répit (cooldown, 60 min par défaut) : une alarme qui ne fait que se maintenir n'est pas republiée pendant ce délai. | Une alarme continue sur 3 h donne 1 message `raised` et au plus 3 rappels `active`. |

**Interface locale**

| ID | Exigence | Critère |
|---|---|---|
| FR-LO-01 | Le C6 émet en BLE en permanence. L'appairage se fait par passkey à 6 chiffres, propre à chaque ruche. | Une connexion sans passkey est refusée. |
| FR-LO-02 | La commande BLE `OPEN_PORTAL` réveille le P4 par une ligne GPIO. Le P4 ouvre alors l'AP Wi-Fi pour au moins 10 min. | L'AP est visible en moins de 15 s après la commande, P4 en deep sleep au départ. |
| FR-LO-03 | Le bouton BOOT ouvre aussi l'AP (comportement v3 conservé). | — |
| FR-LO-04 | La page de gestion affiche : statut (LTE, SD, C6, versions, dernier cycle), appareils Zigbee (valeurs, LQI, batterie, dernière activité), réglages du nœud de pesée avec les écritures en attente, réglages audio et alarme, liste des WAV avec téléchargement, réglages généraux. | Revue manuelle sur Android Chrome et iOS Safari. |
| FR-LO-05 | L'AP se ferme 10 min après la dernière requête HTTP, ou tout de suite par un bouton « Fermer ». | — |
| FR-LO-06 | Mise à jour du firmware P4 en local : envoi d'un `.bin` depuis la page, avec retour arrière si le nouveau firmware ne démarre pas. | Un firmware qui plante 3 fois au boot provoque le retour à la version précédente. |

**Capteurs locaux**

| ID | Exigence | Critère |
|---|---|---|
| FR-SE-01 | BME680 (I2C, GPIO7/8) conservé et détecté au démarrage. S'il est absent, ses champs sont `null`. | Le P4 démarre sans BME680. |

### 1.4 Exigences non fonctionnelles

| ID | Catégorie | Exigence |
|---|---|---|
| NFR-EN-01 | Énergie | Sur 72 h de cycles de 5 min, Wi-Fi fermé, consommation ≤ 30 Wh mesurée à la batterie, C6 permanent compris. Formule : `Wh_cycle × 864 ≤ 30`. |
| NFR-EN-02 | Énergie | Hors mode portail, Wi-Fi et BLE du C6 interne **inactifs** pendant la phase modem, à cause de l'interaction SDIO/USB observée en v3 (`CYCLE_SKIP_MESH_WINDOW`). Le mode portail fait exception, et cette exception est validée par T-INT-07. |
| NFR-RE-01 | Robustesse | Watchdog matériel. Un cycle ne dépasse jamais son intervalle : l'échéance v3 `CYCLE_RESERVE_MS` est conservée. |
| NFR-RE-02 | Robustesse | Si le C6 est absent ou muet, la publication LTE continue, avec `zigbee.online=false`. Si la SD est absente, la publication continue sans file d'attente. |
| NFR-RE-03 | Robustesse | Une coupure d'alimentation pendant l'écriture d'un WAV ou de la file ne corrompt pas le système de fichiers au-delà du fichier en cours. |
| NFR-SE-01 | Sécurité | Mot de passe Wi-Fi (PSK WPA2/WPA3-SAE) propre à chaque ruche, provisionné, **jamais écrit dans les logs ni dans git**. Le mot de passe codé en dur de la v3 disparaît. |
| NFR-SE-02 | Sécurité | Les API qui modifient l'état exigent un jeton de session (obtenu par mot de passe admin), un `Origin` contrôlé et l'absence de CORS `*`. |
| NFR-SE-03 | Sécurité | Toute donnée venant d'un appareil (nom, modèle, valeurs) est échappée en JSON côté serveur. Elle est insérée par `textContent`, jamais par `innerHTML`. |
| NFR-SE-04 | Sécurité | Friendly names limités à `[A-Za-z0-9_-]{1,32}`, pour empêcher toute injection dans les topics (`/ + #`). |
| NFR-SE-05 | Sécurité | Release : Secure Boot v2, chiffrement de la flash et NVS chiffrée sur le P4. La clé mTLS sort du binaire (§2.10). |
| NFR-SE-06 | Sécurité | Les trames UART du C6 sont bornées : longueur maximale, schéma validé, rejet sans effet de bord. |
| NFR-MA-01 | Maintenabilité | La logique métier (converters, protocole, DSP, règles, payloads) est dans une crate `no_std`-compatible, testée sur PC avec `cargo test`, sans matériel. |
| NFR-MA-02 | Maintenabilité | Un seul binaire P4 pour toute la flotte : l'identité de la ruche est provisionnée, et non compilée. |
| NFR-MA-03 | Maintenabilité | Les versions des outils et des dépendances sont épinglées (§3.2). |
| NFR-OB-01 | Observabilité | `bridge/state` publie à chaque cycle : versions P4/C6, uptime, cause du dernier reset, compteurs d'erreurs (UART, LTE, SD) et taille de la file. |

### 1.5 Contraintes et hypothèses

- Carte : Waveshare ESP32-P4-NANO, flash de 16 Mo. Le Wi-Fi 6 et le BLE passent par le C6 **interne**, en SDIO, via `esp_hosted`. Ce C6 interne n'est **pas** utilisé pour le Zigbee.
- Modem : SIMCom **A7672E** (variante Europe, bande B20) sur l'USB-A OTG. Il reste alimenté entre les cycles (`MDM_KEEP_POWERED=1`, pour les raisons documentées en v3).
- C6 externe : **Seeed Studio XIAO ESP32-C6** (4 Mo de flash, USB-C natif, commutateur d'antenne intégrée ou U.FL). Il est alimenté en permanence par sa broche 5V (§2.3.1).
- Le P4 n'a pas de radio 802.15.4 : un C6 externe est donc indispensable au Zigbee.
- La consommation du P4 pendant la fenêtre audio (CPU + I2S, modem au repos) est une inconnue, mesurée au spike S1.

---

## 2. DESIGN

### 2.1 Architecture système

```
 Capteurs Zigbee (T/H, contact, lux, CO2…)
        │ 802.15.4
        ▼
┌──────────────────────────┐   UART 115200 (GPIO24 TX / GPIO23 RX P4)   ┌──────────────────────────────┐
│ ESP32-C6 externe (C)     │◄──────────────────────────────────────────►│ ESP32-P4-NANO (Rust)         │
│ • coordinateur ZBOSS     │   ligne de réveil (C6 → P4 GPIO5, actif 0) │ • pont Z2M-like (converters) │
│ • interview + reporting  │───────────────────────────────────────────►│ • audio INMP441 → DSP, WAV   │
│ • tampon des reports     │                                            │ • SD : file MQTT, audio, logs│
│ • BLE (passkey)          │◄── smartphone (OPEN_PORTAL)                │ • LTE PPP + MQTT mTLS        │
│ • toujours alimenté      │                                            │ • portail Wi-Fi + page web   │
└──────────────────────────┘                                            │ • deep sleep entre cycles    │
                                                                        └──┬──────────┬──────────┬─────┘
                                            I2S GPIO20/21/22 ── INMP441 ───┘          │ SDMMC    │ USB-A OTG
                                            I2C GPIO7/8      ── BME680                │ carte SD │ A7672E ──4G──► Broker MQTT cloud
```

Principe : **le Zigbee ne dépend jamais de l'éveil du P4.** Le P4 est un consommateur périodique. Le C6 détient l'état radio.

### 2.2 Ce que l'on reprend de Zigbee2MQTT (analyse par rétro-ingénierie)

| Concept Zigbee2MQTT | Rôle chez Z2M | Équivalent V27 | Écart volontaire |
|---|---|---|---|
| `zigbee-herdsman` + adaptateur (ezsp/zstack/zboss…) | Pilote la pile radio, interview, bind, configuration des reports | **Firmware C6** (C, `esp-zigbee-lib`) | Interview simplifiée : clusters de la table §2.4 uniquement |
| `zigbee-herdsman-converters` : `fromZigbee` | Traduit les messages ZCL en propriétés JSON | Crate `ruche-core::zcl` : tables `(cluster, attribut) → (propriété, échelle, unité)` | Uniquement les clusters ZCL standard. Pas de code par appareil sauf exception. |
| `toZigbee` | Commandes MQTT→appareil | Table `toZigbee` limitée au modèle `CCA-RUCHE-WEIGHT` (§2.3.2), écritures mises en file dans le C6 (§2.4) | Rien ne descend du cloud ; seul le portail local écrit |
| Device definitions (`zigbeeModel`, `vendor`, `exposes`) | Décrit ce que l'appareil expose | Exposes déduits des clusters vus à l'interview. Table optionnelle `model → surcharges`. | Pas de base de 4000 appareils |
| `database.db` / `configuration.yaml` (`friendly_name`) | État persistant | Côté C6 : NVS (appareils, interview). Côté P4 : NVS (noms, réglages). | — |
| Topics `zigbee2mqtt/<friendly_name>` | Un état JSON par appareil | `ruche/<HIVE_ID>/zigbee/<friendly_name>` | Préfixe par ruche (ACL du broker) |
| `bridge/state`, `bridge/devices`, `bridge/event` | Santé du pont, inventaire, événements | `ruche/<HIVE_ID>/bridge/state`, `…/bridge/devices` (retained), `…/bridge/event` | — |
| `availability` | Appareil en ligne ou non | `last_seen` + `available` (calcul par timeout : 2× l'intervalle max de report) | — |
| Frontend web | Gestion | Page embarquée, servie sur l'AP du P4 | Une seule page, sans framework |

Méthode de validation de cette inspiration : appairer **les mêmes appareils** à une instance Zigbee2MQTT de référence (PC + dongle), capturer les deux sorties et les comparer (T-INT-04).

### 2.3 Firmware C6 (C, ESP-IDF v5.5, `esp-zigbee-lib`)

Répertoire `zigbee-c6/`. Modules :

| Fichier | Rôle |
|---|---|
| `main/zb_coord.c` | Formation et reprise du réseau, permit join, sortie d'un appareil, réinitialisation du réseau |
| `main/zb_interview.c` | Sur `DEVICE_ANNCE` : lecture du Basic (`0x0004` fabricant, `0x0005` modèle, `0x0007` source d'alimentation), Active EP, Simple Desc ; bind + Configure Reporting pour les clusters de §2.4 |
| `main/report_ring.c` | Anneau de 512 entrées `{seq u32, ieee u64, ep u8, cluster u16, attr u16, type u8, value i32/u32/float, lqi u8, t_ms u32}` en RAM. En cas de débordement, l'entrée la plus ancienne est écrasée et `dropped` est incrémenté. |
| `main/dev_table.c` | 32 appareils maximum : IEEE, adresse courte, fabricant, modèle, endpoints, clusters, dernier LQI, last_seen. Persistés en NVS (namespace `zbdev`). |
| `main/uart_proto.c` | Protocole §2.4 : lecture d'une ligne bornée à 512 octets, réponses |
| `main/ble_portal.c` | NimBLE, service GATT `RUCHE-CTRL`, une caractéristique en écriture chiffrée (passkey). Commande `OPEN_PORTAL` → impulsion basse de 50 ms sur la ligne de réveil, puis événement UART `portal_request`. |

Coexistence 802.15.4 + BLE sur le C6 : le coexisteur d'ESP-IDF (`CONFIG_ESP_COEX_SW_COEXIST_ENABLE`) est validé au spike S3. Le C6 reste coordinateur et non routeur dormant : il n'entre pas en sommeil profond.

#### 2.3.1 Carte Seeed Studio XIAO ESP32-C6

| Fonction | Broche XIAO | GPIO C6 | Note |
|---|---|---|---|
| UART1 TX → P4 GPIO23 | D4 | GPIO22 | — |
| UART1 RX ← P4 GPIO24 | D5 | GPIO23 | — |
| Réveil P4 → P4 GPIO5 | D2 | GPIO2 | Sortie open-drain, actif bas, impulsion de 50 ms |
| Activation du commutateur RF | — | GPIO3 | Mis à 0 au boot, sinon aucune antenne n'est raccordée |
| Choix de l'antenne | — | GPIO14 | 0 = antenne céramique intégrée, 1 = U.FL externe. Kconfig `RUCHE_C6_EXT_ANTENNA`, actif par défaut au rucher |
| LED utilisateur | — | GPIO15 (actif bas) | Clignote tant que l'inclusion est ouverte |
| Console, flash | USB-C | USB-Serial-JTAG | La console ne passe pas par l'UART du P4 |

Pourquoi D6/D7 (GPIO16/17, UART0) ne servent pas : la ROM de démarrage y écrit son log à chaque reset. Le P4 rejetterait ces lignes, mais elles n'ont rien à faire sur le lien.

Alimentation :
- Le C6 est alimenté par la broche **5V** du XIAO, depuis le rail 5 V permanent, **en amont** du MOSFET modem (GPIO46). Couper le modem ne doit pas couper le Zigbee.
- Son LDO intégré fournit le 3,3 V ; on n'injecte rien sur la broche 3V3.
- Pour flasher par USB-C sur place, débrancher d'abord le 5 V externe. L'isolation entre VBUS et la broche 5V est à vérifier sur le schéma Seeed au spike S3.

Le partitionnement Zigbee (`nvs`, `zb_storage`, `zb_fct`, `factory`) tient dans les 4 Mo.

#### 2.3.2 Nœud de pesée (projet `ESP32-C6-BalanceRuche`)

Firmware écrit à part, dans le dossier `balance-c6/` de ce dépôt (spec `balance-c6/docs/superpowers/specs/2026-10-23-balance-ruche-zigbee-design.md`). Le concentrateur le reconnaît sans code spécifique pour la mesure ; seuls ses réglages passent par la table `toZigbee` (§2.5.3).

**Matériel**

| Élément | Référence |
|---|---|
| Microcontrôleur | Seeed Studio XIAO ESP32-C6, firmware Arduino (core esp32 3.3.8, mode Zigbee ED) |
| Convertisseur | Adafruit NAU7802, 24 bits, I2C `0x2A` sur D4/D5 (GPIO22/23). LDO interne à 3,0 V, gain ×128, 10 SPS, power-down entre deux mesures |
| Capteur | Variohm AL6N-C3-100kg : 2,0 mV/V, pont de 350 Ω, câble 6 fils (sense raccordé à l'excitation), soit environ 21 475 points/kg |
| Batterie | **Li-Ion 1S** (3,0 à 4,2 V) sur les pastilles BAT du XIAO, rechargée par l'USB-C |
| Interrupteur | Sur le + batterie, avant la pastille BAT+. Sur arrêt, le nœud est hors tension et la batterie ne se décharge pas, mais l'USB-C ne la recharge plus |
| Mesure batterie | Pont 200 kΩ / 200 kΩ vers A0 (GPIO0), 100 nF. En dessous de 3,30 V, la radio n'est pas démarrée (garde anti-brownout) |

**Modèle Zigbee (contrat)**

| EP | Cluster serveur | Description `0x001C` | Accès | Rôle |
|---|---|---|---|---|
| 1 | Basic `0x0000` | — | R / W | `0x0004` = `CCA`, `0x0005` = `CCA-RUCHE-WEIGHT`, source = batterie ; `0x0010` LocationDescription = nom (1 à 16 caractères ASCII imprimables, `ruche-XXXX` par défaut) |
| 1 | Analog Input `0x000C` | `weight` | R, report | `0x0055` PresentValue (float, **kg**), `0x0075` EngineeringUnits = 39 |
| 1 | Power Config `0x0001` | — | R, report | `0x0021` batterie % (unité 0,5 %) ; `0x0020` tension (unité 100 mV), **non reportable** |
| 2 | Analog Output `0x000D` | `tare` | W | Toute écriture déclenche la tare |
| 3 | Analog Output | `calibration_kg` | W | Masse connue posée (> 0) : calcul du facteur |
| 4 | Analog Output | `interval_s` | R / W | Période de réveil, 60 à 86 400 s, 3600 par défaut |
| 5 | Analog Output | `offset` | R / W | Zéro brut en points ADC, \|offset\| ≤ 2²³ |
| 6 | Analog Output | `scale` | R / W | Points ADC par kg, \|scale\| ≥ 1000, 21 475 par défaut |

Une écriture invalide est refusée par le nœud, qui republie l'ancienne valeur.

**Comportement**

- **Cycle normal** (réveil par timer, toutes les `interval_s`) : pesée (4 échantillons jetés, moyenne de 10), mesure batterie, reprise du réseau Zigbee, report du poids et de la batterie avec acquittement (3 essais). Ensuite **fenêtre d'écoute de 5 s**, prolongée de 5 s à chaque écriture reçue, puis deep sleep. Éveil total d'environ 4 s.
- **Mode réglage** : toute mise sous tension, donc aussi le **passage de l'interrupteur sur marche**, ou un RESET, garde le nœud éveillé **180 s** avec une pesée et un report toutes les 10 s. C'est le moment de l'appairage, de la tare et de l'étalonnage : les écritures arrivent immédiatement.
- **Appui de 10 s sur BOOT** : le nœud quitte le réseau (factory reset Zigbee) ; ses réglages de pesée sont conservés.
- **Le nœud n'est joignable que pendant ses fenêtres d'éveil.** Il annonce `rx_on_when_idle = true`, donc son parent ne garde rien pour lui : une écriture envoyée pendant son sommeil est perdue. D'où la file du C6.
- **Autonomie** : environ 0,15 mAh par heure à 3600 s, soit 8 à 9 mois par 1000 mAh de batterie (estimation du projet balance, à mesurer).

**Côté concentrateur**

- **Interview** : le C6 lit aussi le Basic `0x0010` (nom) et les EP4 à EP6 (`interval_s`, `offset`, `scale`), et les transmet dans la ligne `devices` (§2.4).
- **Reporting** : la configuration de reporting est envoyée comme pour tout appareil, mais elle n'a aucun effet sur un nœud en deep sleep. Il reporte à chaque réveil. L'échec du Configure Reporting sur `0x0020` est toléré.
- **File d'écritures** : le C6 garde les écritures du portail et les envoie dès qu'il reçoit un report de ce nœud, donc pendant sa fenêtre d'écoute (§2.4).
- **Choix du nœud de la ruche** : l'appareil indiqué par `weight_ieee` (§2.9) fournit `weight_kg` dans `data`. Si `weight_ieee` est vide et qu'**un seul** appareil `CCA-RUCHE-WEIGHT` est appairé, c'est lui qui est retenu ; s'il y en a plusieurs, le portail demande d'en choisir un.
- Tous les nœuds de pesée, y compris ceux d'autres ruches, publient aussi sur `zigbee/<friendly_name>` (`{"weight":…,"battery":…,"interval_s":…}`).
- `weight_kg` vaut `null` si la dernière mesure est plus ancienne que `weight_max_age_s`. Le portail signale un nœud dont `interval_s` dépasse `weight_max_age_s`.
- **Nom par défaut** : le nom du nœud (`0x0010`) devient son `friendly_name` par défaut s'il respecte NFR-SE-04 et n'est pas déjà pris. Sinon, l'adresse IEEE est utilisée.

#### 2.3.3 Clusters pris en charge

Clusters pris en charge pour l'interview et le reporting (min 60 s, max 900 s, sauf mention) :

| Cluster | Attribut | Report on change |
|---|---|---|
| `0x0001` Power Config | `0x0020` tension, `0x0021` batterie % | max 3600 s |
| `0x0402` Temperature | `0x0000` | Δ 0,2 °C (20) |
| `0x0405` Humidity | `0x0000` | Δ 1 % (100) |
| `0x0403` Pressure | `0x0000` | Δ 1 hPa |
| `0x0400` Illuminance | `0x0000` | Δ 500 |
| `0x0406` Occupancy | `0x0000` | 0 s / 900 s |
| `0x0500` IAS Zone | notification de changement d'état de zone (commande) | évènementiel |
| `0x0006` On/Off | `0x0000` | 0 s / 900 s |
| `0x040D` CO2 | `0x0000` (float) | Δ 50 ppm |
| `0x042A` PM2.5 | `0x0000` (float) | Δ 5 µg/m³ |
| `0x000C` Analog Input (pesée, §2.3.2) | `0x0055` (float) | min 300 s / max 3600 s, Δ 0,1 kg. Sans effet sur le nœud de pesée, qui reporte à chaque réveil |

### 2.4 Protocole UART P4↔C6 v2

- 115200 bit/s, 8N1, 3,3 V, résistances série de 1 kΩ, masse commune.
- **Une ligne JSON par message**, terminée par `\n`, 512 octets au maximum. Une ligne trop longue ou invalide est rejetée par `{"err":"frame"}` (NFR-SE-06).
- Le P4 est maître : il envoie une requête, le C6 répond. Le C6 n'émet spontanément que `{"ev":…}`, et le P4 vide ces lignes avant chaque requête.
- Chaque requête porte un `id` (u16), renvoyé à l'identique dans la réponse. Délai d'attente côté P4 : 500 ms par ligne.

| Requête P4 → C6 | Réponse(s) C6 → P4 |
|---|---|
| `{"id":1,"cmd":"status"}` | `{"id":1,"fw":"27.01.0","ch":15,"pan":"0x1A62","epan":"0x…","devices":3,"permit_join":0,"seq":10234,"dropped":0,"uptime_s":…}` |
| `{"id":2,"cmd":"devices"}` | N lignes `{"id":2,"ieee":"0x00124b…","nwk":"0x3c1f","mfr":"LUMI","model":"lumi.weather","power":"battery","eps":[1],"clusters":[1026,1029,1027,1],"lqi":180,"last_seen_s":42}`, puis `{"id":2,"end":3}` |
| `{"id":3,"cmd":"reports","after":10200,"max":64}` | Lignes `{"id":3,"seq":10201,"ieee":"0x…","ep":1,"cl":1026,"at":0,"ty":41,"v":2150,"lqi":176,"age_s":122}`, puis `{"id":3,"end":10264,"more":true}`. Le P4 boucle tant que `more` vaut true. |
| `{"id":4,"cmd":"permit_join","s":180}` | `{"id":4,"ok":true}` ; si `s` est hors de 1–254 : `{"id":4,"err":"range"}` |
| `{"id":5,"cmd":"remove","ieee":"0x…"}` | `{"id":5,"ok":true}` |
| `{"id":6,"cmd":"reset_network","ch":20,"pan":"0x1B00","confirm":"RESET"}` | `{"id":6,"ok":true}`, puis le C6 redémarre et forme le nouveau réseau |
| `{"id":7,"cmd":"set_ble_passkey","pk":"123456"}` | `{"id":7,"ok":true}` |
| `{"id":8,"cmd":"write","ieee":"0x…","ep":2,"cl":13,"at":85,"ty":57,"v":1.0}` | `{"id":8,"ok":true,"queued":1}` ; file pleine : `{"id":8,"err":"queue_full"}` |
| `{"id":9,"cmd":"write","ieee":"0x…","ep":1,"cl":0,"at":16,"ty":66,"s":"ruche-r1"}` | Idem, pour un attribut chaîne (Basic `0x0010`) |
| `{"id":10,"cmd":"pending"}` | N lignes `{"id":10,"ieee":"0x…","ep":2,"cl":13,"at":85,"age_s":40}`, puis `{"id":10,"end":N}` |
| `{"id":11,"cmd":"cancel_writes","ieee":"0x…"}` | `{"id":11,"ok":true}` |

**File d'écritures du C6** (FR-ZB-09) :
- 16 entrées au total, 6 au maximum par appareil.
- Une nouvelle écriture sur le même `(ieee, ep, cl, at)` remplace l'ancienne.
- Expiration après 48 h.
- La file est en RAM : un redémarrage du C6 la vide (R-10), et le portail affiche alors une file vide.
- Les écritures d'un appareil partent dès qu'un report de cet appareil est reçu, dans l'ordre d'arrivée.

La ligne `devices` porte en plus des champs optionnels lus à l'interview : `zone_type` (IAS), `analog_units`, `location` (Basic `0x0010`, 16 caractères au maximum) et `cfg`. Pour le nœud de pesée, `cfg` vaut `{"interval_s":3600,"offset":…,"scale":…}`.

Événements C6 → P4 : `{"ev":"joined","ieee":…}`, `{"ev":"left","ieee":…}`, `{"ev":"interview","ieee":…,"ok":true}`, `{"ev":"portal_request"}`, `{"ev":"write_done","ieee":…,"ep":2,"cl":13,"at":85,"ok":true}`. `ok` vaut false si le nœud a répondu par un statut ZCL d'erreur, ou si l'écriture a expiré.

`age_s` est calculé par le C6 au moment de l'envoi. Le P4 en déduit un horodatage absolu (`now - age_s`), sans que le C6 ait besoin de connaître l'heure.

### 2.5 Firmware P4 (Rust)

#### 2.5.1 Organisation

```
firmware-p4/
├── Cargo.toml                 # workspace
├── rust-toolchain.toml        # channel "esp" (espup), cible riscv32imafc-esp-espidf
├── ruche-core/                # crate sans dépendance ESP, testée sur PC
│   └── src/ { zcl.rs, c6_proto.rs, audio.rs, alarm.rs, payload.rs, config.rs, names.rs }
└── ruche-fw/                  # binaire ESP-IDF (esp-idf-svc, std)
    ├── build.rs / sdkconfig.defaults / partitions.csv
    └── src/ { main.rs, cycle.rs, c6_link.rs, mic.rs, sd.rs, lte.rs, mqtt.rs,
               portal.rs, portal.html, env_sensor.rs, power.rs, settings.rs, provision.rs }
```

Il n'y a que deux crates, parce que la logique doit pouvoir être testée sans matériel (NFR-MA-01). On n'en ajoute pas d'autre.

Composants C d'Espressif, chargés via `esp-idf-sys` (`[package.metadata.esp-idf-sys] extra_components`) : `esp_hosted`, `esp_wifi_remote`, `usb_host_cdc_acm`, `esp_modem` (PPP).

#### 2.5.2 Modules `ruche-fw`

| Module | Rôle | Dépend de |
|---|---|---|
| `main.rs` | Cause du réveil, initialisation, choix cycle / portail | tous |
| `cycle.rs` | Séquence §2.7 et échéance du cycle | tous |
| `c6_link.rs` | UART1 (GPIO24/23), envoi des requêtes, lecture de lignes bornée → `ruche_core::c6_proto` | `esp-idf-svc::uart` |
| `mic.rs` | I2S0 std RX, 32 bits par slot → échantillons i32 >> 8, tampon de trames 4096 | `esp-idf-svc::i2s` |
| `sd.rs` | Montage SDMMC 4 bits (GPIO39–44, alimentation LDO4 + GPIO45 actif bas, repris de v3), FATFS avec LFN, écriture WAV, file MQTT, rotation | `esp-idf-sys` fatfs |
| `lte.rs` | Alimentation du modem (GPIO46), USB CDC, PPP via `esp_modem` (DTE USB), délais v3 reportés tels quels (`MDM_BOOT_DELAY_MS`…) | `esp_modem`, `usb_host_cdc_acm` |
| `mqtt.rs` | `EspMqttClient` (esp-mqtt), mTLS, QoS0 pour la télémétrie et QoS1 pour les alarmes, LWT `bridge/state = offline` | `esp-idf-svc::mqtt` |
| `portal.rs` | Wi-Fi AP (`esp_wifi_remote`), `EspHttpServer`, API REST §2.8, `portal.html` intégré par `include_str!` | `esp-idf-svc::http` |
| `env_sensor.rs` | BME680, compensation reprise de `sensors.c` v3 | I2C0 |
| `power.rs` | Deep sleep : réveil par timer + GPIO0 (BOOT) + GPIO5 (C6), niveau bas, `esp_deep_sleep_enable_gpio_wakeup` comme en v3 | `esp-idf-sys` |
| `settings.rs` | Réglages en NVS (namespace `ruche27`), validés par `ruche_core::config` | `EspNvs` |
| `provision.rs` | Lecture de la partition `provision` (HIVE_ID, cert, clé, PSK Wi-Fi, mot de passe admin, passkey BLE) | NVS chiffrée |

#### 2.5.3 `ruche-core` (sans ESP, `cargo test`)

- `zcl.rs` : table `toZigbee` du modèle `CCA-RUCHE-WEIGHT`, `fn to_zigbee(model, prop, value) -> Result<Write, Refus>`. Elle traduit `tare`, `calibration_kg`, `interval_s`, `offset`, `scale` et `name` en `(ep, cluster, attribut, type, valeur)`, avec les mêmes bornes que le nœud (§2.3.2) : on refuse dès le portail plutôt qu'au réveil du nœud.
- `zcl.rs` : `fn convert(cluster, attr, ty, raw) -> Option<(Prop, Value)>`, à partir d'une table statique :
  - `0x0402/0` → `temperature` = raw/100 °C ;
  - `0x0405/0` → `humidity` = raw/100 % ;
  - `0x0403/0` → `pressure` hPa ;
  - `0x0400/0` → `illuminance` = 10^((raw−1)/10000) lx ;
  - `0x0001/0x21` → `battery` = raw/2 % ;
  - `0x0001/0x20` → `voltage` = raw×100 mV ;
  - `0x0406/0` → `occupancy` bool ;
  - `0x0500` zone status, bit 0 → `contact` (inversé) ou `water_leak` selon le type de zone ;
  - `0x0006/0` → `state` ON/OFF ;
  - `0x040D/0` → `co2` = float×1e6 ppm ;
  - `0x042A/0` → `pm25` ;
  - `0x000C/0x55` → `weight` en kg, si EngineeringUnits = 39 ou si le modèle est `CCA-RUCHE-WEIGHT`. Sinon : `analog_value`, sans unité.

  Les valeurs « invalides » du ZCL (`0x8000`, `0xFFFF`) sont ignorées.
- `c6_proto.rs` : sérialisation des requêtes, parsing des réponses avec `serde_json` et validation de schéma (bornes, types).
- `audio.rs` : fenêtre de Hann, FFT réelle de 4096 points (`microfft`, `no_std`), RMS en dBFS + `spl_offset_db` (120 par défaut, valeur de la v3), pic entre 50 et 2000 Hz, 4 bandes. Le WAV n'est pas ici : c'est un simple en-tête de 44 octets écrit par `sd.rs`.
- `alarm.rs` : machine d'états `Normal → Suspect(n) → Alarm → Recovering(n) → Normal`, avec `confirm_windows`, `clear_windows` et `cooldown`. L'état est conservé à travers le deep sleep (sérialisé en RTC RAM, et en NVS à chaque transition).
- `payload.rs` : construction de tous les payloads MQTT §2.6.
- `config.rs` : bornes de chaque réglage, valeurs par défaut.
- `names.rs` : validation des friendly names `[A-Za-z0-9_-]{1,32}`, unicité.

Dépendances ajoutées : `serde`, `serde_json`, `microfft`, `heapless`. Rien d'autre sans justification écrite.

### 2.6 Topics et payloads MQTT

Préfixe `ruche/<HIVE_ID>/`. Télémétrie en QoS0 ; alarmes et `bridge/devices` en QoS1.

| Topic | Retained | Payload (exemple) |
|---|---|---|
| `data` | non | `{"hive_id":"cca-ruche-0001","timestamp":1792540800,"fw":"27.01.0","temperature":24.1,"humidity":58,"pressure":951,"weight_kg":41.85,"sound_hz":248,"db_level":62.3,"audio":{"bands":[0.21,0.52,0.19,0.08],"state":"normal","window_s":30},"zigbee":{"online":true,"devices":3,"available":3}}` |
| `zigbee/<friendly_name>` | non | `{"temperature":21.5,"humidity":64.2,"battery":87,"voltage":2950,"linkquality":176,"last_seen":1792540678}`. Une seule publication par appareil et par cycle : la dernière valeur de chaque propriété. |
| `bridge/state` | oui | `{"state":"online","fw_p4":"27.01.0","fw_c6":"27.01.0","uptime_s":…,"reset_reason":"deepsleep","err":{"uart":0,"lte":1,"sd":0},"queue":0,"zb_dropped":0}`. LWT : `{"state":"offline"}`. |
| `bridge/devices` | oui | `[{"ieee":"0x…","friendly_name":"meteo_rucher","manufacturer":"LUMI","model":"lumi.weather","power":"battery","exposes":["temperature","humidity","pressure","battery"]}]`. Publié quand la liste change. |
| `bridge/event` | non | `{"type":"device_joined","ieee":"0x…"}` / `device_left` / `interview_failed` |
| `alarm` | non, QoS1 | `{"type":"acoustic","state":"raised","since":1792540500,"rule":"freq_max","metrics":{"db":71.0,"f_dom":452,"ratio_b3_b2":2.4},"windows":2,"recording":"audio/20261021/101500_alarm.wav"}`. `state` vaut `raised`, `active` ou `cleared`. |

Horodatage : SNTP sur PPP à chaque cycle. L'horloge RTC continue de tourner pendant le deep sleep. Sans heure valide (premier boot, sans réseau), `timestamp` vaut `null` et le message mis en file porte un `uptime_ms`.

### 2.7 Cycle de fonctionnement (5 min par défaut)

```
réveil (timer | BOOT | GPIO5 C6)
 ├─ 1. init minimale, lecture provision + réglages, échéance = t0 + intervalle − CYCLE_RESERVE
 ├─ 2. réveil par BOOT ou par le C6 (portal_request) ? → MODE PORTAIL (§2.8), puis retour au cycle
 ├─ 3. en parallèle :
 │     a. fenêtre audio (30 s) → métriques par trame → verdict sur la fenêtre → machine d'alarme
 │        → WAV sur la SD selon le mode
 │     b. BME680
 │     c. C6 : status, puis reports après last_seq (persisté en RTC/NVS), puis devices si le compteur a changé
 ├─ 4. construction des payloads → file d'attente en mémoire (alarme en tête)
 ├─ 5. Wi-Fi et BLE coupés (NFR-EN-02) → modem : PPP → SNTP → MQTT mTLS
 │     → publication : file SD (FIFO, alarmes d'abord), puis messages du cycle → déconnexion
 │     en cas d'échec ou d'échéance atteinte : écriture dans la file SD
 ├─ 6. rotation audio, log de cycle sur la SD
 └─ 7. deep sleep (intervalle − durée du cycle, minimum CYCLE_MIN_SLEEP_SEC)
```

`last_seq` n'avance qu'une fois les messages publiés **ou** écrits dans la file SD. Une perte en cours de cycle ne fait donc perdre aucun report : ils sont relus au cycle suivant, tant que l'anneau du C6 ne les a pas écrasés.

### 2.8 Interface locale

**Activation**
1. L'appli smartphone (nRF Connect, ou toute appli BLE générique) s'appaire au C6 avec le passkey.
2. Elle écrit `OPEN_PORTAL` dans la caractéristique.
3. Le C6 tire GPIO5 du P4 à l'état bas pendant 50 ms et émet `{"ev":"portal_request"}`.
4. Le P4 se réveille et ouvre l'AP `CCA-<HIVE_ID>` en WPA2/WPA3 avec le PSK provisionné.
5. L'utilisateur ouvre `http://192.168.4.1`.

Si l'appareil le permet, une page Web Bluetooth minimale servie par le P4 peut remplacer nRF Connect, mais ce n'est pas exigé.

**Pendant le portail**
- Le P4 reste éveillé. Les cycles de mesure et de publication continuent à l'intervalle normal.
- Wi-Fi et modem fonctionnent en même temps, comme en mode interactif v3. Ce point est vérifié par T-INT-07 (risque R-03).

**API REST** (JSON ; jeton `Authorization: Bearer` obtenu par `POST /api/login` avec le mot de passe admin ; vérification de `Origin` ; pas de CORS)

| Méthode et chemin | Rôle |
|---|---|
| `POST /api/login` | → `{"token":…}` (valable 15 min) |
| `GET /api/status` | LTE, SD (espace libre), C6, versions, dernier cycle, file d'attente |
| `GET /api/zigbee/devices` | Appareils, dernières valeurs converties, LQI, `last_seen` |
| `POST /api/zigbee/permit_join` `{"s":180}` | Ouvre l'inclusion |
| `POST /api/zigbee/devices/{ieee}/set` `{"tare":true}`, `{"calibration_kg":10.0}`, `{"interval_s":3600}`, `{"offset":…}`, `{"scale":…}`, `{"name":"ruche-r1"}` | Réglage du nœud de pesée, validé par la table `toZigbee`, puis mis en file dans le C6. Réponse : `{"queued":["tare"]}` |
| `GET /api/zigbee/pending` | Écritures en attente de délivrance |
| `PUT /api/zigbee/devices/{ieee}` `{"friendly_name":…}` | Renomme l'appareil |
| `DELETE /api/zigbee/devices/{ieee}` | Exclut l'appareil |
| `GET/POST /api/zigbee/network` | Canal et PAN ; `reset_network` avec `confirm` |
| `GET/PUT /api/settings` | Intervalle, audio, alarme, enregistrement, quota |
| `GET /api/audio/live` | Métriques de la dernière trame (rafraîchies toutes les 1 s par la page) |
| `GET /api/recordings` / `GET /api/recordings/{path}` / `DELETE …` | Liste, téléchargement, suppression des WAV |
| `POST /api/ota` | Envoi d'un `.bin` (partition OTA inactive), puis redémarrage |
| `POST /api/portal/close` | Ferme l'AP, puis deep sleep |

Page : `portal.html` unique (HTML + CSS + JS natif), avec 5 onglets : Statut, Zigbee, Audio, Enregistrements, Réglages. Elle ne charge rien d'externe, car il n'y a pas d'Internet au rucher. Les données des appareils sont insérées par `textContent` (NFR-SE-03).

### 2.9 Réglages (NVS `ruche27`)

| Clé | Défaut | Bornes |
|---|---|---|
| `interval_s` | 300 | 60, 300, 900, 1800, 3600, 7200, 14400, 21600, 43200, 86400 |
| `audio_window_s` | 30 | 5–120 |
| `spl_offset_db` | 120,0 | 80–140 (étalonnage) |
| `lvl_min_db` / `lvl_max_db` | 40 / 85 | 20–120, min < max |
| `f_min_hz` / `f_max_hz` | 150 / 400 | 50–2000, min < max |
| `ratio_b3_b2_max` | 1,5 | 0,1–10 |
| `breach_ratio` | 0,6 | 0,1–1 |
| `confirm_windows` / `clear_windows` | 2 / 2 | 1–10 |
| `alarm_cooldown_min` | 60 | 5–1440 |
| `rec_mode` | `on_alarm` | `off`, `on_alarm`, `always` |
| `rec_quota_mb` | 1024 | 64–(taille de la SD × 0,9) |
| `portal_idle_min` | 10 | 2–60 |
| `weight_ieee` | vide (sélection auto, §2.3.2) | IEEE d'un appareil appairé |
| `weight_max_age_s` | 7200 | 600–86400. Doit rester supérieur ou égal à l'`interval_s` du nœud de pesée (3600 par défaut), sinon `weight_kg` vaut `null` entre deux réveils |

Les valeurs par défaut de la consigne sont un **point de départ** : une colonie normale bourdonne vers 200–300 Hz, et le « piping » ou le pré-essaimage se situe vers 400–500 Hz. Elles s'ajustent par ruche avec la page Audio en direct (§6.3).

### 2.10 Stockage

**Partitions P4 (16 Mo)**

| Nom | Type | Taille | Rôle |
|---|---|---|---|
| `nvs` | data/nvs | 24 Ko | Réglages, noms, état d'alarme, `last_seq` |
| `nvs_keys` | data/nvs_keys | 4 Ko | Clés de chiffrement de la NVS |
| `provision` | data/nvs | 16 Ko | Identité, cert/clé mTLS, PSK, mots de passe (écrite une fois à l'usine) |
| `otadata` | data/ota | 8 Ko | — |
| `phy_init` | data/phy | 4 Ko | — |
| `ota_0` / `ota_1` | app | 4 Mo chacune | Firmware Rust (l'empreinte esp-idf-svc + hosted dépasse probablement les 3 Mo de la v3) |

La partition `factory` disparaît, puisque l'OTA locale exige deux slots. La clé mTLS n'est plus intégrée au binaire. Avec le chiffrement de la flash (NFR-SE-05), la partition `provision` est protégée.

**Carte SD**

```
/sdcard/
├── queue/      000123.json …   file MQTT (un message par fichier : topic + payload + qos)
├── audio/YYYYMMDD/HHMMSS_<alarm|sample>.wav
└── log/cycle.log               une ligne CSV par cycle, rotation à 1 Mo (cycle.1.log)
```

Le format de la file reste celui de la v3 (`storage.c`), avec en plus le topic et la QoS. Écriture atomique : on écrit `*.tmp`, puis on le renomme. Requiert `CONFIG_FATFS_LFN_HEAP=y`, car la v3 est en `LFN_NONE`.

**Partitions C6** : `nvs`, `zb_storage`, `zb_fct` (exigées par `esp-zigbee-lib`), `factory`.

### 2.11 Câblage V27

| Signal | P4-NANO | Autre côté | Note |
|---|---|---|---|
| UART TX | GPIO24 | XIAO D5 (GPIO23) | 1 kΩ en série |
| UART RX | GPIO23 | XIAO D4 (GPIO22) | 1 kΩ en série |
| Réveil P4 | **GPIO5** (LP GPIO, pull-up) | XIAO D2 (GPIO2, open-drain, actif bas) | Libéré par le retrait du HX711 ; capacité de réveil par GPIO LP **à vérifier au spike S1** |
| INMP441 SD / SCK / WS | GPIO20 / 21 / 22 | INMP441 | L/R à GND (canal gauche) |
| BME680 SDA / SCL | GPIO7 / 8 | BME680 | Adresse 0x76 |
| BOOT | GPIO0 | — | Réveil, ouverture du portail |
| Alimentation modem | GPIO46 | MOSFET | Maintenu en deep sleep (v3) |
| SD | GPIO39–44, 45 | onboard | — |
| Modem | USB-A OTG | A7672E | 5 V, 3 A en pointe |
| C6 externe (XIAO) | — | broche 5V ← rail 5 V permanent, en amont du MOSFET modem | Masse commune avec le P4 ; brochage côté C6 au §2.3.1 |

GPIO6 devient libre (ancien SCK du HX711).

### 2.12 Gestion des erreurs

| Défaillance | Détection | Réaction |
|---|---|---|
| C6 muet | Délai d'attente UART dépassé sur `status` | `zigbee.online=false`, compteur `err.uart`, le cycle continue |
| Trame C6 invalide | Échec du parsing ou du schéma | Ligne ignorée, compteur incrémenté, `last_seq` inchangé |
| Anneau du C6 débordé | `dropped` augmente | Publié dans `bridge/state` ; signal qu'il faut raccourcir l'intervalle |
| Modem ou PPP en échec | Échéance du cycle, mêmes tentatives qu'en v3 (`MDM_INIT_RETRIES`) | File SD, deep sleep normal |
| Broker refuse le mTLS | Erreur de connexion | File SD, `err.lte` incrémenté, pas de retry dans le même cycle |
| SD absente ou pleine | Échec du montage ou de l'écriture | Publication directe uniquement, audio non enregistré, signalé dans `bridge/state` |
| I2S muet (micro débranché) | RMS = 0 sur toute la fenêtre | `audio.state="mic_fault"`, **pas** d'alarme acoustique, mais `bridge/event` `mic_fault` |
| Firmware OTA défaillant | Rollback du bootloader (`CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE`) | Retour au slot précédent ; validation de l'image après un cycle complet réussi |
| Panique, watchdog | Cause du reset | Comptée et publiée dans `bridge/state.reset_reason` |

### 2.13 Alternatives rejetées

| Alternative | Raison du rejet |
|---|---|
| Firmware NCP d'Espressif + pile hôte Rust sur le P4 (option B) | Impose un P4 toujours éveillé ; incompatible avec NFR-EN-01 |
| Utiliser le C6 interne pour le Zigbee | Il fait tourner le firmware `esp_hosted`, qui n'expose pas la radio 802.15.4 au P4. De plus, il est éteint quand le P4 dort, ce qui ferait tomber le réseau Zigbee. |
| Web Bluetooth comme seule interface | Ne fonctionne pas sur Safari iOS (décision 1) |
| Écoute continue | Budget énergie (décision 2) |
| Porter en Rust le tunnel AT + mbedTLS + MQTT écrit à la main de la v3 | PPP + esp-mqtt donne QoS1, LWT et mTLS standard. Le tunnel AT reste le **plan de repli** si S2 échoue. |
| Embassy / `esp-hal` (`no_std`) sur le P4 | Support du P4 incomplet ; il faudrait réécrire `esp_hosted`, l'USB host CDC et FATFS. ESP-IDF les fournit déjà. |

---

## 3. BUILD

### 3.1 Arborescence du dépôt (branche `RUCHE-CCA-V27.01`)

```
concentrateur-p4/
├── firmware-p4/     V27 Rust (§2.5)
├── zigbee-c6/       V27 C (§2.3)
├── tools/           provision.py (génère la partition provision), wav_check.py
└── docs/            specs, plans, procédures de test et de déploiement
```

### 3.2 Chaîne d'outils (épinglée)

| Élément | Version |
|---|---|
| ESP-IDF | v5.5.x (même version que la v3), pour le P4 comme pour le C6 |
| Rust | toolchain `esp` installée par `espup`, cible `riscv32imafc-esp-espidf` ; versions exactes figées dans `rust-toolchain.toml` à l'issue de S1 |
| Crates ESP | `esp-idf-svc`, `esp-idf-hal`, `esp-idf-sys` en versions exactes (`=x.y.z`) ; `Cargo.lock` commité |
| Zigbee | `espressif/esp-zigbee-lib` et `esp-zboss-lib` `~1.6.0` |
| Flash | `espflash` (P4), `idf.py` (C6) |

### 3.3 Spikes bloquants (avant tout lot)

Chaque spike produit un prototype jetable et une note go / no-go dans `docs/spikes/`.

| ID | Question | Go si | Sinon |
|---|---|---|---|
| S1 | Le Rust std (`esp-idf-svc`) tourne-t-il sur le P4 avec `esp_hosted` (Wi-Fi AP + HTTP), I2S, SDMMC et le réveil par GPIO5 ? Quelle consommation pendant la fenêtre audio ? | AP + page « hello » + 10 s d'I2S écrites en WAV sur la SD + réveil GPIO5 depuis le deep sleep, consommation mesurée | Hybride : `main` en C qui appelle une lib Rust `staticlib` (`ruche-core`), drivers en C repris de la v3 |
| S2 | PPP par `esp_modem` sur l'A7672E en USB, sur le P4 ? | PPP up + MQTT mTLS publié sur le broker, 20 cycles sur 20 | Portage en Rust du tunnel AT + mbedTLS de la v3 (`modem.c`) via FFI mbedTLS |
| S3 | Coexistence ZBOSS coordinateur + NimBLE sur le C6 ? | 5 appareils Zigbee qui envoient leurs reports pendant 1 h de connexions BLE répétées, sans perte | BLE retiré du C6 ; activation par le BOOT seul, ou par le BLE du C6 interne pendant les fenêtres d'éveil |

### 3.4 Lots d'incréments

Ordre imposé par les dépendances. Chaque lot se termine par ses tests (§4) et un commit.

| Lot | Contenu | Exigences | Dépend de |
|---|---|---|---|
| L1 | `ruche-core` : `config`, `names`, `zcl`, `c6_proto`, `payload` + tests sur PC | FR-ZB-05, FR-MQ-02/03, NFR-SE-04/06 | — |
| L2 | `ruche-core` : `audio`, `alarm` + tests sur PC (signaux synthétiques) | FR-AU-02/03, FR-AL-01..04 | — |
| L3 | C6 : coordinateur, `dev_table`, interview, `report_ring`, `uart_proto` + tests Unity | FR-ZB-01..04, 07, 08 | S3 |
| L4 | P4 : `power`, `settings`, `provision`, `sd` (file + WAV + rotation), `mic`, `env_sensor` | FR-AU-01/04/05, FR-SE-01, NFR-MA-02 | S1, L2 |
| L5 | P4 : `lte`, `mqtt`, `cycle` (sans Zigbee) → parité fonctionnelle avec la v3 hors HX711 et mesh | FR-MQ-01/04/05, FR-AL-03, NFR-RE-01 | S2, L4 |
| L6 | P4 : `c6_link` + intégration du pont Zigbee dans le cycle | FR-ZB-04..06, NFR-RE-02 | L1, L3, L5 |
| L7 | C6 : `ble_portal` + ligne de réveil ; P4 : `portal` + `portal.html` + API + OTA | FR-LO-01..06, FR-ZB-07, NFR-SE-01..03 | L6 |
| L8 | Profil release : Secure Boot, chiffrement flash et NVS ; `tools/provision.py` | NFR-SE-05 | L7 |

### 3.5 Conventions

- Commits Conventional Commits (`feat(p4): …`, `feat(c6): …`, `test(core): …`), en petites unités, sur `RUCHE-CCA-V27.01`. Merge sur `main` après la recette §4.5.
- `cargo fmt` + `cargo clippy -D warnings` sur les deux crates ; `cargo test -p ruche-core` doit passer avant chaque commit.
- Le code et les commentaires suivent le style v3 : commentaires en français qui expliquent le **pourquoi**, constantes de temporisation commentées avec leur mesure d'origine.
- Secrets : jamais dans git (`*.key` déjà ignoré), uniquement dans la partition `provision`.

---

## 4. TEST

### 4.1 Stratégie

| Niveau | Où | Outil | Quand |
|---|---|---|---|
| Unitaire | PC | `cargo test` (ruche-core), Unity (`zigbee-c6/test_apps`) | Chaque commit |
| Intégration | Banc HIL : P4 + C6 + appareils Zigbee + broker de test | Scripts Python (`tools/`), Mosquitto local en mTLS | Fin de chaque lot |
| Système | Banc, longue durée | Wattmètre et enregistreur USB | Avant la recette |
| Recette | 1 ruche réelle | Plateforme cloud | Avant le merge sur `main` |

### 4.2 Tests unitaires (sur PC)

| ID | Cible | Cas |
|---|---|---|
| T-UT-01 | `zcl` | Chaque entrée de la table, valeurs limites, valeurs invalides ZCL ignorées, cluster inconnu → `None`, Analog Input en kg → `weight`, Analog Input sans unité → `analog_value` |
| T-UT-02 | `c6_proto` | Réponses valides, ligne de plus de 512 octets, JSON tronqué, champ hors bornes, `id` différent, `more` en pagination |
| T-UT-03 | `names` | Caractères `/ + # espace é` refusés, longueur 0 ou 33, doublon |
| T-UT-04 | `audio` | Sinus 250 Hz → pic à ±8 Hz ; sinus 450 Hz → ratio b3/b2 > 1,5 ; silence → `mic_fault` ; échelle dB : −6 dBFS → `spl_offset − 6` |
| T-UT-05 | `alarm` | Séquences de fenêtres → transitions attendues ; cooldown ; état sérialisé puis relu à l'identique |
| T-UT-06 | `payload` | `data` conforme au schéma v3 (champs et types, `weight_kg` compris) ; `weight_kg` à `null` si la mesure est périmée ou si aucun nœud n'est retenu ; sélection auto avec 0, 1 ou 2 nœuds de pesée ; échappement JSON de valeurs hostiles (`"`, `<script>`) |
| T-UT-07 | `config` | Chaque borne de §2.9 ; min ≥ max refusé |
| T-UT-09 | `zcl::to_zigbee` | Chaque réglage du nœud de pesée : valeur valide traduite en `(ep, cl, at, ty)` ; refus de `calibration_kg ≤ 0`, de `\|scale\| < 1000`, d'un `interval_s` hors de 60–86 400 et d'un nom vide, trop long ou non imprimable ; propriété inconnue ou autre modèle refusés |
| T-UT-08 | C6 `uart_proto` | `permit_join` 0, 1, 254, 255 ; `reset_network` sans `confirm` → refusé |

### 4.3 Tests d'intégration (banc)

Matériel : P4-NANO, C6 externe, 3 appareils de référence (T/H type Aqara WSDCGQ11LM ou Sonoff SNZB-02, contact IAS, lux), un dongle avec Zigbee2MQTT sur PC, Mosquitto en mTLS, un haut-parleur et un sonomètre.

| ID | Scénario | Attendu |
|---|---|---|
| T-INT-01 | Appairage via `permit_join 180` | `bridge/event device_joined`, puis interview OK, puis premier report en moins de 2 min (FR-ZB-03) |
| T-INT-02 | Coupure du C6, puis remise sous tension | Réseau repris, inclusion fermée (FR-ZB-01/02) |
| T-INT-03 | P4 en deep sleep 5 min, 3 appareils qui envoient leurs reports | Tous les reports relus au réveil, `dropped=0` (FR-ZB-04) |
| T-INT-04 | **Comparaison avec Zigbee2MQTT** : mêmes appareils capturés par les deux chaînes pendant 1 h | Propriétés identiques à l'arrondi près (FR-ZB-05) |
| T-INT-05 | C6 débranché | `data` publié avec `zigbee.online=false` (NFR-RE-02) |
| T-INT-06 | BLE `OPEN_PORTAL` sans passkey, puis avec | Refusé, puis AP visible en moins de 15 s (FR-LO-01/02) |
| T-INT-07 | Portail ouvert pendant 3 cycles avec publication LTE | 3 publications OK, aucune déconnexion USB du modem (R-03) |
| T-INT-08 | Haut-parleur 250 Hz à 60 dB, puis 450 Hz à 75 dB pendant 3 cycles | Pas d'alarme, puis `raised` au 2e cycle, WAV présent, puis `cleared` au retour à 250 Hz (FR-AL-*) |
| T-INT-09 | Broker coupé pendant 2 h, alarme déclenchée pendant la coupure | Au retour du réseau, l'alarme est publiée en premier, puis toute la file (FR-MQ-04) |
| T-INT-10 | SD remplie jusqu'au quota | Rotation : les WAV les plus anciens sont supprimés (FR-AU-05) |
| T-INT-11 | OTA d'un firmware qui panique au boot | Rollback, `reset_reason` publié (FR-LO-06) |
| T-INT-12 | Page : un appareil dont le modèle contient `<img src=x onerror=alert(1)>` (simulé côté C6), une requête POST sans jeton, une requête avec `Origin` étranger | Texte affiché tel quel, puis 401, puis 403 (NFR-SE-02/03) |
| T-INT-13 | 32 appareils simulés (un second C6 en générateur de reports) | Pas de perte sur 1 h (FR-ZB-08) |
| T-INT-14 | Nœud de pesée `ESP32-C6-BalanceRuche` appairé en mode réglage, puis interrupteur sur arrêt | `weight_kg` présent dans `data` et `zigbee/<nom>`, puis `null` après `weight_max_age_s` (FR-MQ-02) |
| T-INT-15 | Depuis le portail, nœud endormi : tare, puis `interval_s = 600`, puis `scale = 0` | Écritures visibles en attente, délivrées au réveil suivant, `write_done ok`. Pesée suivante à vide : 0 ± 0,05 kg. Nouvelle période observée de 600 s. `scale = 0` refusé dès le portail (FR-ZB-09) |
| T-INT-16 | Nœud nommé `ruche-r1`, puis un second nœud nommé `ruche r2` (espace) | Le premier est publié sur `zigbee/ruche-r1`, le second sous son adresse IEEE (NFR-SE-04) |

### 4.4 Tests système

| ID | Test | Critère |
|---|---|---|
| T-SYS-01 | 72 h de cycles de 5 min, Wi-Fi fermé, wattmètre à la batterie | ≤ 30 Wh (NFR-EN-01) |
| T-SYS-02 | 7 jours en continu sur le banc | Aucun reset non planifié ; échéance de cycle toujours respectée (NFR-RE-01) |
| T-SYS-03 | 50 coupures d'alimentation aléatoires pendant l'écriture SD | La SD monte toujours ; au plus le fichier en cours est perdu (NFR-RE-03) |
| T-SYS-04 | Calibrage du micro au sonomètre (94 dB à 1 kHz) | `spl_offset_db` ajusté, erreur ≤ 3 dB |

### 4.5 Recette terrain

Une ruche réelle pendant 14 jours, avec 2 capteurs Zigbee au rucher. Critères :

- au moins 98 % des cycles publiés (le reste est repris depuis la file) ;
- aucune alarme acoustique non justifiée après l'ajustement de la consigne ;
- portail ouvert avec succès 5 fois sur 5, iOS et Android ;
- la plateforme cloud reçoit `data`, `zigbee/*`, `bridge/*` et `alarm`.

### 4.6 Matrice de traçabilité

| Exigence | Design | Tests |
|---|---|---|
| FR-ZB-01/02 | §2.3 `zb_coord`, §2.4 | T-UT-08, T-INT-02 |
| FR-ZB-03 | §2.3 `zb_interview` | T-INT-01 |
| FR-ZB-04 | §2.3 `report_ring`, §2.7 | T-INT-03 |
| FR-ZB-05 | §2.2, §2.5.3 `zcl` | T-UT-01, T-INT-04 |
| FR-ZB-06 | §2.5.3 `names`, §2.8 | T-UT-03 |
| FR-ZB-07 | §2.4, §2.8 | T-UT-08, revue manuelle |
| FR-ZB-08 | §2.3 | T-INT-13 |
| FR-ZB-09 | §2.3.2, §2.4, §2.5.3, §2.8 | T-UT-09, T-INT-15, T-INT-16 |
| FR-MQ-01 | §2.5.2 `lte`/`mqtt` | S2, T-INT-09 |
| FR-MQ-02/03 | §2.3.2, §2.6 | T-UT-01, T-UT-06, T-INT-14 |
| FR-MQ-04 | §2.7, §2.10 | T-INT-09 |
| FR-MQ-05 | §2.9 | T-UT-07 |
| FR-AU-01..03 | §2.5.3 `audio`, §2.9 | T-UT-04, T-SYS-04 |
| FR-AU-04/05 | §2.10 | T-INT-08, T-INT-10 |
| FR-AL-01..04 | §2.5.3 `alarm`, §2.6 | T-UT-05, T-INT-08 |
| FR-LO-01..03 | §2.3 `ble_portal`, §2.8 | T-INT-06 |
| FR-LO-04/05 | §2.8 | Revue manuelle §4.5 |
| FR-LO-06 | §2.10, §2.12 | T-INT-11 |
| FR-SE-01 | §2.5.2 `env_sensor` | Démarrage sans BME680 (L4) |
| NFR-EN-01/02 | §2.7 | T-SYS-01 |
| NFR-RE-01..03 | §2.7, §2.12 | T-SYS-02, T-INT-05, T-SYS-03 |
| NFR-SE-01..06 | §2.8, §2.10, §2.5.3 | T-INT-12, T-UT-02/03/06 |
| NFR-MA-01..03 | §2.5.1, §3.2 | CI `cargo test` |
| NFR-OB-01 | §2.6 `bridge/state` | T-INT-05, T-INT-11 |

---

## 5. DEPLOY

### 5.1 Provisioning (une fois par ruche, à l'atelier)

1. Côté serveur, générer le certificat client pour `<HIVE_ID>` (procédure inchangée : `make client C=<HIVE_ID>`).
2. Lancer `tools/provision.py --hive <HIVE_ID> --cert … --key … --ca …`. Le script génère au hasard le PSK Wi-Fi (20 caractères), le mot de passe admin et le passkey BLE, et produit `provision-<HIVE_ID>.bin` (image NVS). Il imprime une **étiquette** à coller sur le boîtier : SSID, PSK, passkey.
3. Flasher le P4 : bootloader + table de partitions + `ota_0` + `provision`. Le XIAO ESP32-C6 se flashe par USB-C avec `idf.py flash`, alimentation 5 V externe débranchée (§2.3.1).
4. Premier démarrage au banc : vérifier `bridge/state online` sur le broker, puis faire un appairage test.
5. Effacer `provision-<HIVE_ID>.bin` du poste ; l'archive des clés reste côté serveur.

Fini le cycle v3 « modifier `config.h` + `CMakeLists.txt` + symboles `asm()` dans `modem.c` + recompiler » pour chaque ruche (NFR-MA-02).

### 5.2 Release

- Tag git `v27.01.N` sur `main` après la recette. La version est intégrée au binaire (`env!("CARGO_PKG_VERSION")` pour le P4, `PROJECT_VER` pour le C6).
- Artefacts : `ruche-fw-27.01.N.bin` (OTA), image complète de flash usine, `zigbee-c6-27.01.N.bin`, `SHA256SUMS`, notes de version (exigences couvertes, changements de payload).
- Les images de release sont **signées** (Secure Boot v2). La clé de signature reste hors du dépôt, sur le poste de release.

### 5.3 Déploiement progressif

1. Banc : 7 jours (T-SYS-02).
2. Ruche pilote : recette §4.5, 14 jours.
3. Reste du rucher : une ruche par jour, avec contrôle de `bridge/state` à J+1.

### 5.4 Migration depuis la v3 et retour arrière

- La v3 a une table de partitions différente (`factory` de 3 Mo) : la première installation V27 se fait **par câble** (flash complète), jamais par OTA.
- Retour arrière vers la v3 : flasher l'image v3 archivée (le build actuel `cca-ruche`). La NVS v3 (`ruche_cfg`, `hx711`) n'est pas reprise.
- Prévenir l'équipe SmartBees avant le pilote : nouveaux topics, `satellites` retiré de `data` (`weight_kg` reste, mais peut valoir `null`), ACL `ruche/<HIVE_ID>/#`.

---

## 6. MAINTAIN

### 6.1 Supervision

- Côté cloud, `bridge/state` (retained + LWT) sert de tableau de bord de santé de la flotte. Seuils d'attention : `queue > 12`, `zb_dropped > 0`, `err.lte` en hausse sur 3 cycles, `reset_reason` ∉ {`deepsleep`, `poweron`}.
- Sur le terrain, `log/cycle.log` sur la SD contient une ligne par cycle : durées des phases, erreurs.

### 6.2 Mises à jour

- **P4** : OTA locale par le portail (FR-LO-06), avec rollback automatique. L'image est validée une fois le premier cycle complet réussi.
- **C6** : flash par USB sur place. La mise à jour du C6 par le P4 (UART + `esp-serial-flasher`) se fera quand la flotte la justifiera.
- **OTA par LTE** : hors périmètre V27. À réévaluer si la flotte dépasse ~10 ruches ou si le terrain est difficile d'accès.

### 6.3 Ajustement de la consigne acoustique

Par saison et par ruche, sur la page Audio en direct : observer le niveau et les bandes de fréquence au calme, puis régler `f_max_hz` et `ratio_b3_b2_max` à environ +20 % au-dessus de ce qu'on observe. Les WAV `on_alarm` servent à confirmer ou infirmer les alarmes et à affiner les réglages.

### 6.4 Prise en charge d'un nouvel appareil Zigbee (même démarche que Zigbee2MQTT)

1. L'appairer au banc. Si ses clusters sont standard, il fonctionne tel quel (exposes déduits).
2. Sinon, capturer les trames `reports` dans un fichier `ruche-core/tests/captures/<model>.jsonl` et comparer avec le converter Zigbee2MQTT du modèle (`zigbee-herdsman-converters`).
3. Ajouter l'entrée dans la table `zcl` (ou une surcharge `model → …`) et un test qui rejoue la capture.
4. Ajouter le cluster au reporting du C6 seulement si nécessaire (§2.3).

### 6.5 Dépendances et sécurité

- Revue trimestrielle : correctifs ESP-IDF 5.5.x, `esp-idf-svc`, `esp-zigbee-lib`, avis de sécurité mbedTLS et NimBLE. Une mise à jour = une branche, les tests §4.2 et §4.3, puis une release mineure.
- Rotation des certificats mTLS avant leur expiration : regénérer la partition `provision`. Seule la partition `provision` est reflashée, l'application ne change pas.

### 6.6 Fin de vie de la v3

Quand toutes les ruches seront en V27 et après 30 jours sans retour arrière : archiver définitivement la copie privée de la v3.

---

## 7. Risques

| ID | Risque | Prob. | Impact | Mitigation |
|---|---|---|---|---|
| R-01 | Rust std, `esp-idf-svc` et `esp_hosted` encore immatures sur le P4 | Moyenne | Fort | Spike S1 + repli hybride C/Rust |
| R-02 | `esp_modem` en USB instable avec l'A7672E sur le P4 (historique v3 chargé) | Moyenne | Fort | Spike S2 + repli : portage du tunnel AT v3 ; délais v3 conservés |
| R-03 | Interaction SDIO (Wi-Fi hosted) / USB host pendant le portail | Moyenne | Moyen | T-INT-07 ; à défaut, suspendre la publication LTE pendant le portail |
| R-04 | Coexistence Zigbee + BLE sur le C6 | Faible | Moyen | Spike S3 + repli sans BLE |
| R-05 | GPIO5 non utilisable pour le réveil du P4 | Faible | Moyen | Vérifié au S1 ; repli : C6 câblé en parallèle du BOOT (GPIO0, open-drain) |
| R-06 | Fausses alarmes acoustiques (vent, pluie, prédateurs) | Élevée | Moyen | Confirmation sur 2 fenêtres, règle de ratio de bandes, WAV pour vérifier, ajustement §6.3 |
| R-07 | Consommateur cloud v3 sensible à `weight_kg = null` ou au retrait de `satellites` | Faible | Faible | Prévenir SmartBees avant le pilote (§5.4) |
| R-09 | Nœud de pesée pas encore recetté (firmware écrit, recette au banc Zigbee2MQTT à faire) | Faible | Faible | Recette du projet balance, puis T-INT-14 à T-INT-16 |
| R-10 | Écritures du portail perdues si le C6 redémarre (file en RAM) ou si le nœud reste éteint plus de 48 h | Moyenne | Faible | Le portail affiche la file (`GET /api/zigbee/pending`) ; il suffit de renvoyer le réglage |
| R-08 | Consommation du P4 pendant la fenêtre audio hors budget | Moyenne | Moyen | Mesure au S1 ; réduire `audio_window_s` ou baisser la fréquence CPU pendant la capture |

## 8. Décisions et hypothèses restantes

| Point | Statut |
|---|---|
| Modèle du C6 externe | **Décidé** : Seeed Studio XIAO ESP32-C6 (§2.3.1) |
| `weight_kg` dans `data` | **Décidé** : conservé, fourni par le nœud de pesée Zigbee (§2.3.2) |
| Réveil du P4 par GPIO5 | **Hypothèse acceptée**, vérifiée au spike S1. Repli : R-05 |
| Isolation VBUS / broche 5V du XIAO | À vérifier sur le schéma Seeed au spike S3 |
| Nœud de pesée | **Décidé** : aligné sur le projet `ESP32-C6-BalanceRuche` (spec du 2026-10-23), alimentation **Li-Ion 1S avec interrupteur** (2026-10-26). La spec du projet balance indique encore une LiPo sans interrupteur et doit être mise à jour |
| Réglages à distance du nœud de pesée | **Décidé** : file d'écritures dans le C6 et table `toZigbee` dans `ruche-core` (FR-ZB-09). À implémenter aux plans 2 et 3 : `ruche-core` (déjà livré) devra recevoir `Request::Write`, `Event::WriteDone`, les champs `location`/`cfg` de `DeviceInfo` et `zcl::to_zigbee` |
