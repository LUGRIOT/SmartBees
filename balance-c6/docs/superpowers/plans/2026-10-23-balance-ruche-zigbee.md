# Balance de ruche Zigbee — Plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal :** firmware Arduino pour XIAO ESP32-C6 qui pèse une ruche (NAU7802 + AL6N-C3-100kg) et la publie en Zigbee selon le contrat V27 §2.3.2, avec des réglages pilotables depuis le coordinateur.

**Architecture :** un sketch Arduino en quatre fichiers.
- `scale_math.h` : la logique pure, testée sur PC avec g++.
- `zb_weight.h` : l'endpoint 1 du contrat (sous-classe de `ZigbeeAnalog`).
- `config.h` : les constantes.
- `BalanceRuche.ino` : la mesure, le cycle, le deep sleep et les écritures.

Le nœud reste éveillé quelques secondes par cycle, puis passe en deep sleep. Les réglages sont des Analog Output standards (EP2 à EP6) et le nom est stocké dans le Basic `0x0010` de l'EP1. Un external converter Zigbee2MQTT sert au banc.

**Tech Stack :** Arduino core esp32 3.3.8 (librairie `Zigbee`, `Preferences`), Adafruit NAU7802 Library, g++ (C++17) pour les tests PC, arduino-cli 1.5.1 (embarqué dans l'IDE) pour la compilation, Zigbee2MQTT 2.x pour le banc.

**Spec :** `docs/superpowers/specs/2026-10-23-balance-ruche-zigbee-design.md`

## Global Constraints

- Core Arduino **esp32 3.3.8**, carte **XIAO_ESP32C6**, *Zigbee mode* **Zigbee ED (end device)**, *Partition scheme* **Zigbee 4MB with spiffs**.
- FQBN : `esp32:esp32:XIAO_ESP32C6:ZigbeeMode=ed,PartitionScheme=zigbee`.
- arduino-cli : `ACLI=<chemin vers l'arduino-cli fourni avec l'IDE Arduino 2>`.
- Basic : fabricant `CCA`, modèle `CCA-RUCHE-WEIGHT`, source d'alimentation = batterie.
- EP1 : Analog Input, Description `weight`, PresentValue en kg (float), EngineeringUnits `0x0075` = 39, Power Config `0x0020`/`0x0021`, Basic `0x0010` = nom.
- EP2 `tare`, EP3 `calibration_kg`, EP4 `interval_s`, EP5 `offset`, EP6 `scale` : Analog Output `0x000D`, PresentValue `0x0055`.
- `poids_kg = (brut − offset) / scale` ; scale par défaut 21 475 ; `|scale| ≥ 1000` exigé.
- `interval_s` : de 60 à 86 400, 3600 par défaut ; sommeil minimal de 10 s.
- Nom : de 1 à 16 caractères ASCII 0x20 à 0x7E ; défaut `ruche-xxxx` (4 derniers caractères hexadécimaux de l'IEEE, en minuscules).
- NVS : namespace `balance`, clés `offset` (float), `scale` (float), `interval` (u32), `name` (string).
- NAU7802 : LDO 3,0 V, gain ×128, 10 SPS, calibration **interne uniquement**, power-down entre les mesures ; 4 échantillons jetés puis moyenne de 10.
- Batterie : pont 200 k/200 k sur A0 (GPIO0), moyenne de 16 lectures, courbe 3300/3600/3700/3800/3950/4200 mV = 0/10/30/60/85/100 %.
- Temps : fenêtre d'écoute de 5 s (prolongée de 5 s à chaque écriture), mode réglage de 180 s avec une pesée toutes les 10 s, appui BOOT de 10 s = factory reset Zigbee, timeout de begin de 10 s, acquittement de report : 3 essais × 1 s.
- Jamais de redémarrage en boucle en cas d'échec réseau : deep sleep, puis nouvel essai au cycle suivant.
- Commentaires du code en français.

## Review Focus

1. **NaN ou ±inf écrits sur un réglage numérique** (Zigbee2MQTT envoie un float) : `offset`, `scale` et `calibration_kg` sont refusés ; `interval_s` revient à la valeur par défaut (3600). Testé en Task 1.
2. **Étalonnage sans charge ou avec une masse ≤ 0** (`brut ≈ offset`, m = 0, m < 0) : refusé et scale inchangé. Un câblage inversé (scale négatif mais `|scale| ≥ 1000`) reste accepté. Testé en Task 1.
3. **Nom vide, de 17 caractères ou plus, ou contenant de l'UTF-8 (« é »)** : refusé. Testé en Task 1.
4. **Lecture batterie hors table** (0 mV quand le pont est absent, 5000 mV sur USB) : donne 0 % et 100 %, jamais de dépassement. Testé en Task 1.
5. **Éveil plus long que l'intervalle** (mode réglage de 180 s avec `interval_s` = 60) : le sommeil vaut 10 s, sans dépassement d'entier non signé. Testé en Task 1.

---

## Structure des fichiers

```
BalanceRuche/
  BalanceRuche.ino   mesure, réglages NVS, cycle, deep sleep, écritures, bouton BOOT
  config.h           broches et constantes de temps et de mesure
  scale_math.h       fonctions pures (aucune dépendance Arduino)
  zb_weight.h        classe WeightEndpoint (EP1 du contrat V27)
tests/
  test_scale_math.cpp
z2m/
  cca_ruche_weight.mjs
.gitignore           build/
```

---

### Task 1 : logique pure `scale_math.h` et ses tests

**Files :**
- Create : `BalanceRuche/scale_math.h`
- Create : `tests/test_scale_math.cpp`
- Create : `.gitignore`

**Interfaces :**
- Consumes : rien.
- Produces (utilisé par les tâches 2 à 4) :
  - constantes : `SCALE_DEFAULT` (21475.0f), `SCALE_MIN_ABS` (1000.0f), `OFFSET_MAX_ABS` (8388608.0f), `INTERVAL_MIN_S` (60), `INTERVAL_MAX_S` (86400), `INTERVAL_DEFAULT_S` (3600), `MIN_SLEEP_S` (10), `NAME_MAX_LEN` (16) ;
  - `float weightKg(int32_t raw, float offset, float scale)` ;
  - `bool scaleValid(float scale)` ;
  - `bool offsetValid(float offset)` ;
  - `bool calibrate(int32_t raw, float offset, float massKg, float *scale)`, qui renvoie false et laisse `*scale` intact en cas de refus ;
  - `uint32_t clampInterval(float v)` ;
  - `uint32_t sleepSeconds(uint32_t interval_s, uint32_t awake_ms)` ;
  - `uint8_t batteryPercent(uint32_t mv)` ;
  - `bool nameValid(const char *s, size_t len)` ;
  - `void defaultName(uint8_t hi, uint8_t lo, char *out)`, avec `out` d'au moins 11 octets.

- [ ] **Step 1 : écrire le test qui échoue**

`.gitignore` :

```
build/
```

`tests/test_scale_math.cpp` :

```cpp
// Tests PC de scale_math.h : g++ -std=c++17 -Wall -Wextra tests/test_scale_math.cpp -o build/test_scale_math && build/test_scale_math
#include <cassert>
#include <cmath>
#include <cstdio>
#include <cstring>
#include "../BalanceRuche/scale_math.h"

static bool near(float a, float b, float eps = 0.001f) { return fabsf(a - b) <= eps; }

int main() {
  // Poids
  assert(near(weightKg(214750 + 1000, 1000.0f, 21475.0f), 10.0f));
  assert(near(weightKg(1000, 1000.0f, 21475.0f), 0.0f));

  // Facteur
  assert(scaleValid(21475.0f));
  assert(scaleValid(-21475.0f));  // câblage S+/S− inversé
  assert(!scaleValid(0.0f));
  assert(!scaleValid(999.0f));
  assert(!scaleValid(NAN));
  assert(!scaleValid(INFINITY));

  // Offset
  assert(offsetValid(0.0f));
  assert(offsetValid(-8388608.0f));
  assert(!offsetValid(9000000.0f));
  assert(!offsetValid(NAN));
  assert(!offsetValid(-INFINITY));

  // Étalonnage
  float s = 1234.0f;
  assert(calibrate(214750, 0.0f, 10.0f, &s) && near(s, 21475.0f));
  s = 1234.0f;
  assert(calibrate(-214750, 0.0f, 10.0f, &s) && near(s, -21475.0f));  // inversé accepté
  s = 1234.0f;
  assert(!calibrate(500, 0.0f, 10.0f, &s) && s == 1234.0f);           // pas de charge
  assert(!calibrate(214750, 0.0f, 0.0f, &s) && s == 1234.0f);
  assert(!calibrate(214750, 0.0f, -1.0f, &s) && s == 1234.0f);
  assert(!calibrate(214750, 0.0f, NAN, &s) && s == 1234.0f);
  assert(!calibrate(214750, 0.0f, INFINITY, &s) && s == 1234.0f);

  // Intervalle
  assert(clampInterval(3600.0f) == 3600);
  assert(clampInterval(10.0f) == 60);
  assert(clampInterval(-5.0f) == 60);
  assert(clampInterval(100000.0f) == 86400);
  assert(clampInterval(299.6f) == 300);
  assert(clampInterval(NAN) == INTERVAL_DEFAULT_S);
  assert(clampInterval(INFINITY) == 86400);

  // Durée de sommeil
  assert(sleepSeconds(3600, 4000) == 3596);
  assert(sleepSeconds(60, 180000) == MIN_SLEEP_S);  // éveil plus long que l'intervalle
  assert(sleepSeconds(60, 55000) == MIN_SLEEP_S);
  assert(sleepSeconds(60, 40000) == 20);

  // Batterie
  assert(batteryPercent(0) == 0);
  assert(batteryPercent(3300) == 0);
  assert(batteryPercent(3450) == 5);
  assert(batteryPercent(3700) == 30);
  assert(batteryPercent(3875) == 72);
  assert(batteryPercent(4200) == 100);
  assert(batteryPercent(5000) == 100);

  // Nom
  assert(nameValid("ruche-03", 8));
  assert(nameValid("Rucher-Nord-R12!", 16));
  assert(!nameValid("", 0));
  assert(!nameValid("abcdefghijklmnopq", 17));
  assert(!nameValid("ruch\xc3\xa9", 6));  // « é » en UTF-8
  assert(!nameValid("a\nb", 3));
  assert(!nameValid(nullptr, 3));

  char n[11];
  defaultName(0xab, 0x0d, n);
  assert(strcmp(n, "ruche-ab0d") == 0);
  assert(nameValid(n, strlen(n)));

  puts("OK scale_math");
  return 0;
}
```

- [ ] **Step 2 : vérifier que le test échoue**

Run : `mkdir -p build && g++ -std=c++17 -Wall -Wextra tests/test_scale_math.cpp -o build/test_scale_math`
Expected : FAIL, `fatal error: ../BalanceRuche/scale_math.h: No such file or directory`

- [ ] **Step 3 : implémentation minimale**

`BalanceRuche/scale_math.h` :

```cpp
#pragma once
// Logique pure de la balance : aucune dépendance Arduino, testée sur PC (tests/test_scale_math.cpp).
#include <math.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>

constexpr float SCALE_DEFAULT = 21475.0f;     // 2 mV/V × 128 × 2^23 / 100 kg (AL6N-C3-100kg)
constexpr float SCALE_MIN_ABS = 1000.0f;      // en dessous : étalonnage absurde (et division par ~0)
constexpr float OFFSET_MAX_ABS = 8388608.0f;  // 2^23 : plage de l'ADC 24 bits signé
constexpr uint32_t INTERVAL_MIN_S = 60;
constexpr uint32_t INTERVAL_MAX_S = 86400;
constexpr uint32_t INTERVAL_DEFAULT_S = 3600;
constexpr uint32_t MIN_SLEEP_S = 10;
constexpr size_t NAME_MAX_LEN = 16;           // Basic 0x0010 LocationDescription

inline float weightKg(int32_t raw, float offset, float scale) {
  return ((float)raw - offset) / scale;
}

inline bool scaleValid(float scale) {
  return isfinite(scale) && fabsf(scale) >= SCALE_MIN_ABS;
}

inline bool offsetValid(float offset) {
  return isfinite(offset) && fabsf(offset) <= OFFSET_MAX_ABS;
}

// Facteur à partir d'une masse connue posée sur le plateau (tare faite avant).
inline bool calibrate(int32_t raw, float offset, float massKg, float *scale) {
  if (!(massKg > 0.0f) || !isfinite(massKg)) return false;  // rejette aussi NaN
  float s = ((float)raw - offset) / massKg;
  if (!scaleValid(s)) return false;
  *scale = s;
  return true;
}

inline uint32_t clampInterval(float v) {
  if (isnan(v)) return INTERVAL_DEFAULT_S;
  if (v < (float)INTERVAL_MIN_S) return INTERVAL_MIN_S;
  if (v > (float)INTERVAL_MAX_S) return INTERVAL_MAX_S;
  return (uint32_t)lroundf(v);
}

// Sommeil = intervalle − durée d'éveil, jamais moins de MIN_SLEEP_S.
inline uint32_t sleepSeconds(uint32_t interval_s, uint32_t awake_ms) {
  uint32_t awake_s = awake_ms / 1000;
  return interval_s > awake_s + MIN_SLEEP_S ? interval_s - awake_s : MIN_SLEEP_S;
}

// LiPo 1S, interpolation linéaire par morceaux.
inline uint8_t batteryPercent(uint32_t mv) {
  static const uint16_t MV[] = {3300, 3600, 3700, 3800, 3950, 4200};
  static const uint8_t PCT[] = {0, 10, 30, 60, 85, 100};
  if (mv <= MV[0]) return 0;
  for (int i = 1; i < 6; i++) {
    if (mv <= MV[i]) return PCT[i - 1] + (PCT[i] - PCT[i - 1]) * (mv - MV[i - 1]) / (MV[i] - MV[i - 1]);
  }
  return 100;
}

// 1 à 16 caractères ASCII imprimables.
inline bool nameValid(const char *s, size_t len) {
  if (!s || len == 0 || len > NAME_MAX_LEN) return false;
  for (size_t i = 0; i < len; i++) {
    if ((uint8_t)s[i] < 0x20 || (uint8_t)s[i] > 0x7E) return false;
  }
  return true;
}

// « ruche-xxxx » : 2 derniers octets de l'adresse IEEE.
inline void defaultName(uint8_t hi, uint8_t lo, char *out) {
  snprintf(out, 11, "ruche-%02x%02x", hi, lo);
}
```

- [ ] **Step 4 : vérifier que le test passe**

Run : `g++ -std=c++17 -Wall -Wextra tests/test_scale_math.cpp -o build/test_scale_math && build/test_scale_math`
Expected : `OK scale_math`, sans avertissement de compilation.

- [ ] **Step 5 : commit**

```bash
git add .gitignore BalanceRuche/scale_math.h tests/test_scale_math.cpp
git commit -m "feat: logique pure de la balance (poids, étalonnage, bornes, batterie, nom)"
```

---

### Task 2 : mesure NAU7802, batterie et réglages NVS (sketch de diagnostic)

**Files :**
- Create : `BalanceRuche/config.h`
- Create : `BalanceRuche/BalanceRuche.ino`

**Interfaces :**
- Consumes : tout `scale_math.h` (Task 1).
- Produces (utilisé par les tâches 3 et 4) :
  - globales : `Adafruit_NAU7802 nau`, `Preferences prefs` (ouvert sur `balance`), `float gOffset`, `float gScale`, `uint32_t gInterval`, `char gName[NAME_MAX_LEN + 1]`, `int32_t gLastRaw`, `bool gHaveRaw` ;
  - `void loadSettings()` ;
  - `bool measure()`, qui met à jour `gLastRaw` et `gHaveRaw` ;
  - `uint32_t readBatteryMv()` ;
  - constantes de `config.h`.
- `setup()` et `loop()` de cette tâche sont **temporaires** (diagnostic série) : la Task 3 les remplace.

- [ ] **Step 1 : installer la librairie Adafruit NAU7802**

Run : `$ACLI lib install "Adafruit NAU7802 Library"`
Expected : `Installed Adafruit NAU7802 Library@…` (Adafruit BusIO est déjà présente).

- [ ] **Step 2 : écrire `config.h`**

```cpp
#pragma once
// Broches et constantes. Les réglages modifiables à distance sont dans scale_math.h et en NVS.
#include <stdint.h>

// Broches XIAO ESP32-C6 (NAU7802 sur l'I2C par défaut : SDA D4/GPIO22, SCL D5/GPIO23)
#define PIN_BATTERY A0       // GPIO0 : point milieu du pont 200 k / 200 k
#define PIN_BOOT    BOOT_PIN // GPIO9 : appui long = factory reset Zigbee

// Zigbee
#define ZB_MANUFACTURER "CCA"
#define ZB_MODEL        "CCA-RUCHE-WEIGHT"
constexpr uint8_t EP_WEIGHT = 1;
constexpr uint8_t EP_TARE = 2;
constexpr uint8_t EP_CALIB = 3;
constexpr uint8_t EP_INTERVAL = 4;
constexpr uint8_t EP_OFFSET = 5;
constexpr uint8_t EP_SCALE = 6;

// Temps
constexpr uint32_t LISTEN_WINDOW_MS = 5000;         // écoute des écritures après chaque report
constexpr uint32_t SETUP_MODE_MS = 180000;          // éveil après mise sous tension / RESET
constexpr uint32_t SETUP_MEASURE_PERIOD_MS = 10000; // pesée en mode réglage
constexpr uint32_t FACTORY_RESET_HOLD_MS = 10000;
constexpr uint32_t ZB_BEGIN_TIMEOUT_MS = 10000;
constexpr uint32_t REPORT_ACK_TIMEOUT_MS = 1000;
constexpr int REPORT_TRIES = 3;

// Mesure
constexpr int NAU_DISCARD = 4;                 // stabilisation après réveil et calibration
constexpr int NAU_SAMPLES = 10;
constexpr uint32_t NAU_SAMPLE_TIMEOUT_MS = 2000;
constexpr int BATTERY_SAMPLES = 16;
constexpr float BATTERY_DIVIDER = 2.0f;        // ajuster si le pont réel n'est pas exactement 1:2
```

- [ ] **Step 3 : écrire `BalanceRuche.ino` (mesure et diagnostic)**

```cpp
// Balance de ruche Zigbee — XIAO ESP32-C6 + Adafruit NAU7802 + Variohm AL6N-C3-100kg
// Spec : docs/superpowers/specs/2026-10-23-balance-ruche-zigbee-design.md
#ifndef ZIGBEE_MODE_ED
#error "Outils > Zigbee mode : choisir « Zigbee ED (end device) »"
#endif

#include <Preferences.h>
#include <Wire.h>
#include <Adafruit_NAU7802.h>
#include <esp_mac.h>
#include "config.h"
#include "scale_math.h"

Adafruit_NAU7802 nau;
Preferences prefs;

// Réglages persistés (NVS « balance »)
float gOffset = 0.0f;
float gScale = SCALE_DEFAULT;
uint32_t gInterval = INTERVAL_DEFAULT_S;
char gName[NAME_MAX_LEN + 1];

// Dernière pesée
int32_t gLastRaw = 0;
bool gHaveRaw = false;

void loadSettings() {
  prefs.begin("balance", false);
  gOffset = prefs.getFloat("offset", 0.0f);
  if (!offsetValid(gOffset)) gOffset = 0.0f;
  gScale = prefs.getFloat("scale", SCALE_DEFAULT);
  if (!scaleValid(gScale)) gScale = SCALE_DEFAULT;
  gInterval = clampInterval((float)prefs.getUInt("interval", INTERVAL_DEFAULT_S));
  String name = prefs.getString("name", "");
  if (nameValid(name.c_str(), name.length())) {
    strcpy(gName, name.c_str());
  } else {
    uint8_t ieee[8];
    esp_read_mac(ieee, ESP_MAC_IEEE802154);
    defaultName(ieee[6], ieee[7], gName);
  }
}

bool readSample(int32_t *v) {
  uint32_t t0 = millis();
  while (!nau.available()) {
    if (millis() - t0 > NAU_SAMPLE_TIMEOUT_MS) return false;
    delay(1);
  }
  *v = nau.read();
  return true;
}

// Réveille le NAU7802, fait la moyenne de NAU_SAMPLES lectures, le remet en power-down.
bool measure() {
  Wire.begin();
  // begin() : reset, LDO 3,0 V, gain 128, 10 SPS, attente de 600 ms
  if (!nau.begin(&Wire)) {
    Serial.println("NAU7802 absent");
    return false;
  }
  // Calibration interne uniquement : CALMOD_OFFSET effacerait la charge posée sur le plateau.
  // Les échantillons jetés couvrent aussi la fin de cette calibration.
  bool ok = nau.calibrate(NAU7802_CALMOD_INTERNAL);
  int32_t v;
  int64_t sum = 0;
  for (int i = 0; ok && i < NAU_DISCARD; i++) ok = readSample(&v);
  for (int i = 0; ok && i < NAU_SAMPLES; i++) {
    ok = readSample(&v);
    sum += v;
  }
  nau.enable(false);  // power-down : LDO coupé, pont de jauge non alimenté
  if (!ok) {
    Serial.println("NAU7802 : pas de donnée");
    return false;
  }
  gLastRaw = (int32_t)(sum / NAU_SAMPLES);
  gHaveRaw = true;
  return true;
}

uint32_t readBatteryMv() {
  uint32_t sum = 0;
  for (int i = 0; i < BATTERY_SAMPLES; i++) sum += analogReadMilliVolts(PIN_BATTERY);
  return (uint32_t)((float)sum / BATTERY_SAMPLES * BATTERY_DIVIDER);
}

// Diagnostic temporaire : remplacé par le cycle Zigbee en Task 3.
void setup() {
  Serial.begin(115200);
  delay(2000);  // le temps d'ouvrir le moniteur série
  loadSettings();
  Serial.printf("Nom %s, offset %.0f, scale %.1f, intervalle %lu s\n", gName, gOffset, gScale, (unsigned long)gInterval);
}

void loop() {
  if (measure()) Serial.printf("brut %ld  poids %.3f kg\n", (long)gLastRaw, weightKg(gLastRaw, gOffset, gScale));
  uint32_t mv = readBatteryMv();
  Serial.printf("batterie %lu mV (%u %%)\n", (unsigned long)mv, batteryPercent(mv));
  delay(2000);
}
```

- [ ] **Step 4 : compiler**

Run : `$ACLI compile --fqbn esp32:esp32:XIAO_ESP32C6:ZigbeeMode=ed,PartitionScheme=zigbee BalanceRuche`
Expected : `Sketch uses … bytes`, sans erreur.

- [ ] **Step 5 : contrôle matériel (fait par l'utilisateur, à noter dans le compte rendu)**

Téléverser, puis ouvrir le moniteur série à 115200 bauds :
- le brut varie quand on appuie sur le plateau ;
- le poids est à peu près juste, à ±10 % près, avec le facteur théorique ;
- la tension batterie est cohérente avec un voltmètre, sinon ajuster `BATTERY_DIVIDER`.

- [ ] **Step 6 : commit**

```bash
git add BalanceRuche/config.h BalanceRuche/BalanceRuche.ino
git commit -m "feat: mesure NAU7802, batterie et réglages NVS (diagnostic série)"
```

---

### Task 3 : endpoints Zigbee, report et cycle de sommeil

**Files :**
- Create : `BalanceRuche/zb_weight.h`
- Modify : `BalanceRuche/BalanceRuche.ino` (includes, globales Zigbee, remplacement de `setup()` et `loop()`)

**Interfaces :**
- Consumes : `loadSettings()`, `measure()`, `readBatteryMv()`, globales et constantes des tâches 1 et 2.
- Produces (utilisé par la Task 4) :
  - `class WeightEndpoint : public ZigbeeAnalog` avec `bool addContractAttributes()`, `bool setName(const char *name)`, `void onNameWrite(void (*cb)(const char *s, size_t len))` ;
  - globales `WeightEndpoint zbWeight`, `ZigbeeAnalog zbTare, zbCalib, zbInterval, zbOffset, zbScale`, `uint32_t gAwakeUntil`, `bool gSetupMode` ;
  - `void publishWeight()`, qui écrit le poids dans l'EP1 et le reporte avec acquittement ;
  - `bool reportAcked(uint16_t cluster)`.

Pas de test PC possible ici : tout dépend de la pile Zigbee. La vérification se fait par compilation, puis au banc (Task 6).

- [ ] **Step 1 : écrire `zb_weight.h`**

```cpp
#pragma once
// Endpoint 1 du contrat V27 §2.3.2 : Analog Input « weight » + EngineeringUnits + nom (Basic 0x0010).
#include "Zigbee.h"
#include "scale_math.h"

class WeightEndpoint : public ZigbeeAnalog {
public:
  explicit WeightEndpoint(uint8_t endpoint) : ZigbeeAnalog(endpoint) {}

  // Ajoute les attributs absents de ZigbeeAnalog. À appeler après addAnalogInput() et avant Zigbee.addEndpoint().
  bool addContractAttributes() {
    esp_zb_attribute_list_t *ai = esp_zb_cluster_list_get_cluster(_cluster_list, ESP_ZB_ZCL_CLUSTER_ID_ANALOG_INPUT, ESP_ZB_ZCL_CLUSTER_SERVER_ROLE);
    uint16_t units = 39;  // BACnet : kilogrammes
    if (esp_zb_analog_input_cluster_add_attr(ai, ESP_ZB_ZCL_ATTR_ANALOG_INPUT_ENGINEERING_UNITS_ID, &units) != ESP_OK) return false;
    // Valeur initiale de 16 espaces : réserve par précaution la taille maximale de la chaîne.
    // Le vrai nom est posé par setName() après Zigbee.begin().
    char zcl[NAME_MAX_LEN + 2];
    zcl[0] = NAME_MAX_LEN;
    memset(zcl + 1, ' ', NAME_MAX_LEN);
    zcl[NAME_MAX_LEN + 1] = '\0';
    esp_zb_attribute_list_t *basic = esp_zb_cluster_list_get_cluster(_cluster_list, ESP_ZB_ZCL_CLUSTER_ID_BASIC, ESP_ZB_ZCL_CLUSTER_SERVER_ROLE);
    return esp_zb_basic_cluster_add_attr(basic, ESP_ZB_ZCL_ATTR_BASIC_LOCATION_DESCRIPTION_ID, zcl) == ESP_OK;
  }

  // Publie le nom dans Basic 0x0010. À appeler après Zigbee.begin().
  bool setName(const char *name) {
    char zcl[NAME_MAX_LEN + 2];
    size_t len = strnlen(name, NAME_MAX_LEN);
    zcl[0] = (char)len;
    memcpy(zcl + 1, name, len);
    esp_zb_lock_acquire(portMAX_DELAY);
    esp_zb_zcl_status_t st = esp_zb_zcl_set_attribute_val(
      _endpoint, ESP_ZB_ZCL_CLUSTER_ID_BASIC, ESP_ZB_ZCL_CLUSTER_SERVER_ROLE, ESP_ZB_ZCL_ATTR_BASIC_LOCATION_DESCRIPTION_ID, zcl, false
    );
    esp_zb_lock_release();
    return st == ESP_ZB_ZCL_STATUS_SUCCESS;
  }

  // Appelé (tâche Zigbee) quand le coordinateur écrit Basic 0x0010 : chaîne non terminée et sa longueur.
  void onNameWrite(void (*cb)(const char *s, size_t len)) {
    _on_name = cb;
  }

private:
  void (*_on_name)(const char *s, size_t len) = nullptr;

  void zbAttributeSet(const esp_zb_zcl_set_attr_value_message_t *message) override {
    if (message->info.cluster == ESP_ZB_ZCL_CLUSTER_ID_BASIC && message->attribute.id == ESP_ZB_ZCL_ATTR_BASIC_LOCATION_DESCRIPTION_ID && _on_name) {
      const uint8_t *v = (const uint8_t *)message->attribute.data.value;  // chaîne ZCL : [longueur][caractères]
      _on_name((const char *)v + 1, v[0]);
    }
  }
};
```

- [ ] **Step 2 : ajouter les includes et les globales Zigbee dans `BalanceRuche.ino`**

Remplacer :

```cpp
#include <Preferences.h>
```

par :

```cpp
#include "Zigbee.h"
#include <Preferences.h>
```

Remplacer :

```cpp
#include "scale_math.h"
```

par :

```cpp
#include "scale_math.h"
#include "zb_weight.h"
```

Remplacer :

```cpp
// Dernière pesée
int32_t gLastRaw = 0;
bool gHaveRaw = false;
```

par :

```cpp
// Dernière pesée
int32_t gLastRaw = 0;
bool gHaveRaw = false;

// Endpoints Zigbee (spec §3)
WeightEndpoint zbWeight(EP_WEIGHT);
ZigbeeAnalog zbTare(EP_TARE);
ZigbeeAnalog zbCalib(EP_CALIB);
ZigbeeAnalog zbInterval(EP_INTERVAL);
ZigbeeAnalog zbOffset(EP_OFFSET);
ZigbeeAnalog zbScale(EP_SCALE);

bool gSetupMode = false;      // démarrage hors timer : éveil prolongé
uint32_t gAwakeUntil = 0;     // millis() de fin d'éveil
uint32_t gLastMeasureMs = 0;
volatile uint16_t gAckCluster = 0;
volatile bool gAcked = false;
```

- [ ] **Step 3 : remplacer `setup()` et `loop()` de diagnostic par le cycle**

Remplacer tout le bloc qui va de `// Diagnostic temporaire : remplacé par le cycle Zigbee en Task 3.` jusqu'à la fin du fichier par :

```cpp
void onResponse(zb_cmd_type_t cmd, esp_zb_zcl_status_t status, uint8_t endpoint, uint16_t cluster) {
  if (cmd == ZB_CMD_REPORT_ATTRIBUTE && endpoint == EP_WEIGHT && cluster == gAckCluster && status == ESP_ZB_ZCL_STATUS_SUCCESS) gAcked = true;
}

// Report d'un cluster de l'EP1, avec attente de l'acquittement (REPORT_TRIES essais).
bool reportAcked(uint16_t cluster) {
  gAckCluster = cluster;
  for (int i = 0; i < REPORT_TRIES; i++) {
    gAcked = false;
    if (cluster == ESP_ZB_ZCL_CLUSTER_ID_ANALOG_INPUT) zbWeight.reportAnalogInput();
    else zbWeight.reportBatteryPercentage();
    uint32_t t0 = millis();
    while (!gAcked && millis() - t0 < REPORT_ACK_TIMEOUT_MS) delay(10);
    if (gAcked) return true;
  }
  Serial.printf("Report 0x%04x non acquitté\n", cluster);
  return false;
}

void publishWeight() {
  if (!gHaveRaw) return;
  float kg = weightKg(gLastRaw, gOffset, gScale);
  zbWeight.setAnalogInput(kg);
  Serial.printf("Poids %.2f kg (brut %ld)\n", kg, (long)gLastRaw);
  reportAcked(ESP_ZB_ZCL_CLUSTER_ID_ANALOG_INPUT);
}

void sleepNow() {
  uint32_t s = sleepSeconds(gInterval, millis());
  Serial.printf("Deep sleep %lu s\n", (unsigned long)s);
  Serial.flush();
  esp_sleep_enable_timer_wakeup((uint64_t)s * 1000000ULL);
  esp_deep_sleep_start();
}

void addParam(ZigbeeAnalog &ep, const char *description) {
  ep.addAnalogOutput();
  ep.setAnalogOutputDescription(description);
}

void setupEndpoints(uint32_t mv) {
  zbWeight.setManufacturerAndModel(ZB_MANUFACTURER, ZB_MODEL);
  zbWeight.setPowerSource(ZB_POWER_SOURCE_BATTERY, batteryPercent(mv), mv / 100);
  zbWeight.addAnalogInput();
  zbWeight.setAnalogInputDescription("weight");
  zbWeight.setAnalogInputResolution(0.01);
  if (!zbWeight.addContractAttributes()) Serial.println("Attributs du contrat V27 non ajoutés");
  addParam(zbTare, "tare");
  addParam(zbCalib, "calibration_kg");
  addParam(zbInterval, "interval_s");
  zbInterval.setAnalogOutputMinMax(INTERVAL_MIN_S, INTERVAL_MAX_S);
  addParam(zbOffset, "offset");
  addParam(zbScale, "scale");
  Zigbee.addEndpoint(&zbWeight);
  Zigbee.addEndpoint(&zbTare);
  Zigbee.addEndpoint(&zbCalib);
  Zigbee.addEndpoint(&zbInterval);
  Zigbee.addEndpoint(&zbOffset);
  Zigbee.addEndpoint(&zbScale);
}

void setup() {
  Serial.begin(115200);
  pinMode(PIN_BOOT, INPUT_PULLUP);
  gSetupMode = esp_sleep_get_wakeup_cause() != ESP_SLEEP_WAKEUP_TIMER;
  loadSettings();
  measure();  // avant la radio : moins de bruit, et le NAU7802 est coupé plus tôt
  uint32_t mv = readBatteryMv();
  setupEndpoints(mv);
  Zigbee.onGlobalDefaultResponse(onResponse);

  esp_zb_cfg_t cfg = ZIGBEE_DEFAULT_ED_CONFIG();
  cfg.nwk_cfg.zed_cfg.keep_alive = 10000;
  Zigbee.setTimeout(ZB_BEGIN_TIMEOUT_MS);
  if (!Zigbee.begin(&cfg, false)) {
    Serial.println("Zigbee : démarrage impossible");
    sleepNow();
  }
  // Déjà appairé : connecté dès le redémarrage. Premier appairage : le temps d'ouvrir l'inclusion.
  uint32_t joinMs = gSetupMode ? SETUP_MODE_MS : ZB_BEGIN_TIMEOUT_MS;
  uint32_t t0 = millis();
  while (!Zigbee.connected()) {
    if (millis() - t0 > joinMs) {
      Serial.println("Zigbee : pas de réseau");
      sleepNow();
    }
    delay(100);
  }

  zbWeight.setName(gName);
  zbInterval.setAnalogOutput(gInterval);
  zbOffset.setAnalogOutput(gOffset);
  zbScale.setAnalogOutput(gScale);
  zbWeight.setBatteryVoltage(mv / 100);
  zbWeight.setBatteryPercentage(batteryPercent(mv));
  publishWeight();
  reportAcked(ESP_ZB_ZCL_CLUSTER_ID_POWER_CONFIG);

  gLastMeasureMs = millis();
  gAwakeUntil = millis() + (gSetupMode ? SETUP_MODE_MS : LISTEN_WINDOW_MS);
}

void loop() {
  if (gSetupMode && millis() - gLastMeasureMs >= SETUP_MEASURE_PERIOD_MS) {
    gLastMeasureMs = millis();
    if (measure()) publishWeight();
  }
  if ((int32_t)(millis() - gAwakeUntil) >= 0) sleepNow();
  delay(10);
}
```

- [ ] **Step 4 : compiler**

Run : `$ACLI compile --fqbn esp32:esp32:XIAO_ESP32C6:ZigbeeMode=ed,PartitionScheme=zigbee BalanceRuche`
Expected : compilation sans erreur. Si l'override de `zbAttributeSet` est refusé (méthode `private` dans `ZigbeeAnalog`), vérifier qu'il est bien déclaré `override` dans la section `private:` de `WeightEndpoint`. En C++, on peut redéfinir une méthode virtuelle privée.

- [ ] **Step 5 : relancer les tests PC (aucune régression)**

Run : `g++ -std=c++17 -Wall -Wextra tests/test_scale_math.cpp -o build/test_scale_math && build/test_scale_math`
Expected : `OK scale_math`

- [ ] **Step 6 : commit**

```bash
git add BalanceRuche/zb_weight.h BalanceRuche/BalanceRuche.ino
git commit -m "feat: endpoints Zigbee du contrat V27, report acquitté et cycle deep sleep"
```

---

### Task 4 : écritures du coordinateur et factory reset

**Files :**
- Modify : `BalanceRuche/BalanceRuche.ino`

**Interfaces :**
- Consumes : `zbWeight` et ses méthodes `onNameWrite`/`setName`, `zbTare`, `zbCalib`, `zbInterval`, `zbOffset`, `zbScale`, `measure()`, `publishWeight()`, `gAwakeUntil`, `prefs`, `gOffset`, `gScale`, `gInterval`, `gName`, `calibrate()`, `clampInterval()`, `offsetValid()`, `scaleValid()`, `nameValid()`.
- Produces : `void attachWriteCallbacks()`, `void applyWrites()`, `void checkFactoryReset()`.

- [ ] **Step 1 : ajouter les callbacks et le traitement des écritures**

Insérer juste avant la ligne `void setup() {` :

```cpp
// Écritures du coordinateur : la tâche Zigbee les note, loop() les applique.
volatile bool wTare = false, wCalib = false, wInterval = false, wOffset = false, wScale = false, wName = false;
volatile float vCalib, vInterval, vOffset, vScale;
volatile bool vNameOk = false;
char vName[NAME_MAX_LEN + 1];

void onTare(float) { wTare = true; }
void onCalib(float v) { vCalib = v; wCalib = true; }
void onInterval(float v) { vInterval = v; wInterval = true; }
void onOffset(float v) { vOffset = v; wOffset = true; }
void onScale(float v) { vScale = v; wScale = true; }

void onName(const char *s, size_t len) {
  vNameOk = nameValid(s, len);
  if (vNameOk) {
    memcpy(vName, s, len);
    vName[len] = '\0';
  }
  wName = true;
}

void attachWriteCallbacks() {
  zbTare.onAnalogOutputChange(onTare);
  zbCalib.onAnalogOutputChange(onCalib);
  zbInterval.onAnalogOutputChange(onInterval);
  zbOffset.onAnalogOutputChange(onOffset);
  zbScale.onAnalogOutputChange(onScale);
  zbWeight.onNameWrite(onName);
}

// Applique les écritures reçues. Une valeur refusée est republiée telle qu'avant.
// Toute écriture prolonge l'éveil de LISTEN_WINDOW_MS.
void applyWrites() {
  if (!(wTare || wCalib || wInterval || wOffset || wScale || wName)) return;
  if (wTare) {
    wTare = false;
    if (measure()) {
      gOffset = (float)gLastRaw;
      prefs.putFloat("offset", gOffset);
      Serial.println("Tare faite");
    }
    zbOffset.setAnalogOutput(gOffset);
    publishWeight();
  }
  if (wCalib) {
    wCalib = false;
    if (measure() && calibrate(gLastRaw, gOffset, vCalib, &gScale)) {
      prefs.putFloat("scale", gScale);
      Serial.printf("Étalonnage : scale = %.1f\n", gScale);
    } else {
      Serial.println("Étalonnage refusé");
    }
    zbScale.setAnalogOutput(gScale);
    publishWeight();
  }
  if (wInterval) {
    wInterval = false;
    gInterval = clampInterval(vInterval);
    prefs.putUInt("interval", gInterval);
    zbInterval.setAnalogOutput(gInterval);
  }
  if (wOffset) {
    wOffset = false;
    if (offsetValid(vOffset)) {
      gOffset = vOffset;
      prefs.putFloat("offset", gOffset);
    }
    zbOffset.setAnalogOutput(gOffset);
    publishWeight();
  }
  if (wScale) {
    wScale = false;
    if (scaleValid(vScale)) {
      gScale = vScale;
      prefs.putFloat("scale", gScale);
    }
    zbScale.setAnalogOutput(gScale);
    publishWeight();
  }
  if (wName) {
    wName = false;
    if (vNameOk) {
      strcpy(gName, vName);
      prefs.putString("name", gName);
    }
    zbWeight.setName(gName);
  }
  uint32_t until = millis() + LISTEN_WINDOW_MS;
  if ((int32_t)(until - gAwakeUntil) > 0) gAwakeUntil = until;
}

// BOOT maintenu FACTORY_RESET_HOLD_MS : quitte le réseau Zigbee et redémarre (réglages NVS conservés).
void checkFactoryReset() {
  if (digitalRead(PIN_BOOT) != LOW) return;
  uint32_t t0 = millis();
  while (digitalRead(PIN_BOOT) == LOW) {
    if (millis() - t0 >= FACTORY_RESET_HOLD_MS) {
      Serial.println("Factory reset Zigbee");
      Zigbee.factoryReset();
    }
    delay(50);
  }
}

```

- [ ] **Step 2 : brancher les callbacks avant `Zigbee.begin()`**

Remplacer :

```cpp
  setupEndpoints(mv);
  Zigbee.onGlobalDefaultResponse(onResponse);
```

par :

```cpp
  setupEndpoints(mv);
  attachWriteCallbacks();
  Zigbee.onGlobalDefaultResponse(onResponse);
```

- [ ] **Step 3 : traiter les écritures et le bouton dans `loop()`**

Remplacer :

```cpp
void loop() {
  if (gSetupMode && millis() - gLastMeasureMs >= SETUP_MEASURE_PERIOD_MS) {
```

par :

```cpp
void loop() {
  applyWrites();
  checkFactoryReset();
  if (gSetupMode && millis() - gLastMeasureMs >= SETUP_MEASURE_PERIOD_MS) {
```

- [ ] **Step 4 : compiler et relancer les tests PC**

Run : `$ACLI compile --fqbn esp32:esp32:XIAO_ESP32C6:ZigbeeMode=ed,PartitionScheme=zigbee BalanceRuche && g++ -std=c++17 -Wall -Wextra tests/test_scale_math.cpp -o build/test_scale_math && build/test_scale_math`
Expected : compilation sans erreur, puis `OK scale_math`.

- [ ] **Step 5 : commit**

```bash
git add BalanceRuche/BalanceRuche.ino
git commit -m "feat: réglages pilotés par le coordinateur (tare, étalonnage, intervalle, offset, scale, nom) et factory reset"
```

---

### Task 5 : external converter Zigbee2MQTT (banc)

**Files :**
- Create : `z2m/cca_ruche_weight.mjs`

**Interfaces :**
- Consumes : le modèle Zigbee des tâches 3 et 4 (EP1 à EP6, Basic `0x0010` = `locationDesc` pour zigbee-herdsman).
- Produces : propriétés MQTT `weight`, `battery`, `voltage`, `tare` (enum `tare`), `calibration_kg`, `interval_s`, `offset`, `scale`, `hive_name`.

- [ ] **Step 1 : écrire le converter**

```js
// External converter Zigbee2MQTT 2.x — balance de ruche CCA (banc de test uniquement).
// Installation : copier dans <data>/external_converters/ puis redémarrer Zigbee2MQTT.
import * as exposes from 'zigbee-herdsman-converters/lib/exposes';

const e = exposes.presets;
const ea = exposes.access;

// Propriété MQTT -> endpoint Analog Output du nœud
const PARAMS = {tare: 2, calibration_kg: 3, interval_s: 4, offset: 5, scale: 6};
// Tare et étalonnage refont une pesée (~1,5 s) puis modifient offset ou scale : c'est eux qu'on relit.
const READ_BACK = {tare: 'offset', calibration_kg: 'scale'};

const fzWeight = {
    cluster: 'genAnalogInput',
    type: ['attributeReport', 'readResponse'],
    convert: (model, msg) => {
        if (msg.endpoint.ID === 1 && msg.data.presentValue !== undefined) {
            return {weight: Math.round(msg.data.presentValue * 100) / 100};
        }
    },
};

const fzParams = {
    cluster: 'genAnalogOutput',
    type: ['attributeReport', 'readResponse'],
    convert: (model, msg) => {
        const key = Object.keys(PARAMS).find((k) => PARAMS[k] === msg.endpoint.ID);
        if (key && !READ_BACK[key] && msg.data.presentValue !== undefined) return {[key]: msg.data.presentValue};
    },
};

const fzName = {
    cluster: 'genBasic',
    type: ['attributeReport', 'readResponse'],
    convert: (model, msg) => {
        if (msg.data.locationDesc !== undefined) return {hive_name: msg.data.locationDesc.trim()};
    },
};

const fzBattery = {
    cluster: 'genPowerCfg',
    type: ['attributeReport', 'readResponse'],
    convert: (model, msg) => {
        const result = {};
        if (msg.data.batteryPercentageRemaining !== undefined) result.battery = Math.round(msg.data.batteryPercentageRemaining / 2);
        if (msg.data.batteryVoltage !== undefined) result.voltage = msg.data.batteryVoltage * 100;
        return result;
    },
};

const tzParams = {
    key: Object.keys(PARAMS),
    convertSet: async (entity, key, value, meta) => {
        const v = key === 'tare' ? 1 : Number(value);
        await meta.device.getEndpoint(PARAMS[key]).write('genAnalogOutput', {presentValue: v});
        // Relit la valeur réellement appliquée par le nœud (bornage, refus).
        const readBack = READ_BACK[key] ?? key;
        if (READ_BACK[key]) await new Promise((resolve) => setTimeout(resolve, 3000));
        await meta.device.getEndpoint(PARAMS[readBack]).read('genAnalogOutput', ['presentValue']);
    },
    convertGet: async (entity, key, meta) => {
        await meta.device.getEndpoint(PARAMS[READ_BACK[key] ?? key]).read('genAnalogOutput', ['presentValue']);
    },
};

const tzName = {
    key: ['hive_name'],
    convertSet: async (entity, key, value, meta) => {
        const ep = meta.device.getEndpoint(1);
        await ep.write('genBasic', {locationDesc: String(value)});
        await ep.read('genBasic', ['locationDesc']);
    },
    convertGet: async (entity, key, meta) => {
        await meta.device.getEndpoint(1).read('genBasic', ['locationDesc']);
    },
};

export default {
    zigbeeModel: ['CCA-RUCHE-WEIGHT'],
    model: 'CCA-RUCHE-WEIGHT',
    vendor: 'CCA',
    description: 'Balance de ruche (NAU7802 + AL6N-C3-100kg)',
    fromZigbee: [fzWeight, fzParams, fzName, fzBattery],
    toZigbee: [tzParams, tzName],
    exposes: [
        e.numeric('weight', ea.STATE).withUnit('kg').withDescription('Poids de la ruche'),
        e.battery(),
        e.battery_voltage(),
        e.enum('tare', ea.SET, ['tare']).withDescription('Remise à zéro (plateau vide)'),
        e.numeric('calibration_kg', ea.SET).withUnit('kg').withValueMin(0.1).withValueMax(150).withDescription('Masse connue posée sur le plateau'),
        e.numeric('interval_s', ea.ALL).withUnit('s').withValueMin(60).withValueMax(86400).withDescription('Période de mesure'),
        e.numeric('offset', ea.ALL).withDescription('Zéro brut (points ADC)'),
        e.numeric('scale', ea.ALL).withDescription('Points ADC par kg'),
        e.text('hive_name', ea.ALL).withDescription('Nom de la ruche (16 caractères max)'),
    ],
    configure: async (device, coordinatorEndpoint) => {
        const ep1 = device.getEndpoint(1);
        await ep1.bind('genAnalogInput', coordinatorEndpoint);
        await ep1.bind('genPowerCfg', coordinatorEndpoint);
        await ep1.read('genBasic', ['locationDesc']);
        await ep1.read('genPowerCfg', ['batteryPercentageRemaining', 'batteryVoltage']);
        for (const key of ['interval_s', 'offset', 'scale']) {
            await device.getEndpoint(PARAMS[key]).read('genAnalogOutput', ['presentValue']);
        }
    },
};
```

- [ ] **Step 2 : vérifier la syntaxe**

Run : `node --check z2m/cca_ruche_weight.mjs`
Expected : aucune sortie, code de sortie 0. Si `node` est absent, le noter dans le compte rendu : la vérification aura lieu au chargement par Zigbee2MQTT (Task 6).

- [ ] **Step 3 : commit**

```bash
git add z2m/cca_ruche_weight.mjs
git commit -m "feat: external converter Zigbee2MQTT pour le banc"
```

---

### Task 6 : recette au banc (faite par l'utilisateur)

**Files :** aucun. Les résultats sont à reporter dans la conversation ; une correction éventuelle fera l'objet d'une tâche à part.

**Préparation :**
1. Dans l'IDE Arduino :
   - carte XIAO_ESP32C6 ;
   - Zigbee mode « Zigbee ED (end device) » ;
   - Partition scheme « Zigbee 4MB with spiffs » ;
   - *Erase All Flash Before Sketch Upload* : « Enabled » au premier téléversement seulement.
2. Copier `z2m/cca_ruche_weight.mjs` dans `<data Zigbee2MQTT>/external_converters/`, redémarrer Zigbee2MQTT et vérifier dans le log que le converter est chargé.
3. Ouvrir l'inclusion (*Permit join*), puis mettre le nœud sous tension. Il passe en mode réglage pour 180 s.

- [ ] **Step 1** : appairage. Zigbee2MQTT reconnaît `CCA-RUCHE-WEIGHT` (vendor `CCA`), l'interview et la configuration réussissent, `hive_name` vaut `ruche-xxxx`, et `xxxx` correspond aux 4 derniers caractères de l'IEEE affichée par Zigbee2MQTT.
- [ ] **Step 2** : `weight` se met à jour toutes les 10 s ; `battery` et `voltage` sont cohérents.
- [ ] **Step 3** : relire l'attribut `0x0075` de `genAnalogInput` (EP1) dans l'onglet *Dev console* : il vaut 39. Relire aussi `description` : il vaut `weight`.
- [ ] **Step 4** : plateau vide, `tare` → après environ 3 s, `offset` change et `weight` vaut 0 ± 0,05 kg.
- [ ] **Step 5** : poser une masse connue, puis écrire `calibration_kg` = sa masse → `scale` change. Avec une seconde masse connue, l'erreur est inférieure ou égale à 0,1 kg.
- [ ] **Step 6** : `interval_s` = 120 → à la fin du mode réglage, le poids remonte toutes les 2 min environ (fenêtre de 5 s visible dans le log série).
- [ ] **Step 7** : `hive_name` = `Rucher-Nord-R12` → relu à l'identique.
- [ ] **Step 8** : écritures invalides refusées et ancienne valeur relue : `scale` = 0, `calibration_kg` = -1, `hive_name` = `` (vide), `hive_name` = `ruche-numero-trop-long`.
- [ ] **Step 9** : coupure d'alimentation, puis remise sous tension → reconnexion sans réappairage ; `offset`, `scale`, `interval_s` et `hive_name` sont conservés.
- [ ] **Step 10** : sans NAU7802 (câble STEMMA débranché) → le log série affiche « NAU7802 absent », la batterie remonte et le nœud dort quand même.
- [ ] **Step 11** : en mode réglage, appui de 10 s sur BOOT → le nœud quitte le réseau, redémarre et peut être réappairé ; `hive_name` et l'étalonnage sont conservés.
- [ ] **Step 12a** : cycle normal sans écriture → le log série affiche « Deep sleep … » environ 5 s après le report (régression : republier un réglage ne doit jamais empêcher le sommeil).
- [ ] **Step 12b** : depuis la *Dev console* (sans passer par le converter), écrire dans `genBasic.locationDesc` de l'EP1 une chaîne de 40 caractères → pas de plantage ni de redémarrage, et l'ancien nom est relu.
- [ ] **Step 12c** : alimentation de labo à 3,2 V sur BAT → le log affiche « Batterie trop faible » et le nœud dort sans démarrer la radio.
- [ ] **Step 12** : consommation en deep sleep mesurée au multimètre sur la batterie : environ 25 µA attendus. Si c'est nettement plus, vérifier la LED de la carte NAU7802.
