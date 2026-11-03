// Balance de ruche Zigbee — XIAO ESP32-C6 + Adafruit NAU7802 + Variohm AL6N-C3-100kg
// Spec : docs/superpowers/specs/2026-10-23-balance-ruche-zigbee-design.md
#ifndef ZIGBEE_MODE_ED
#error "Outils > Zigbee mode : choisir « Zigbee ED (end device) »"
#endif

#include "Zigbee.h"
#include <Preferences.h>
#include <Wire.h>
#include <Adafruit_NAU7802.h>
#include <esp_mac.h>
#include "config.h"
#include "scale_math.h"
#include "zb_weight.h"

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

// Republie un réglage (Analog Output PresentValue) sans passer par ZigbeeAnalog::setAnalogOutput(),
// qui rappelle le callback d'écriture : le nœud croirait recevoir une écriture et ne dormirait plus.
void publishParam(uint8_t endpoint, float value) {
  esp_zb_lock_acquire(portMAX_DELAY);
  esp_zb_zcl_set_attribute_val(
    endpoint, ESP_ZB_ZCL_CLUSTER_ID_ANALOG_OUTPUT, ESP_ZB_ZCL_CLUSTER_SERVER_ROLE, ESP_ZB_ZCL_ATTR_ANALOG_OUTPUT_PRESENT_VALUE_ID, &value, false
  );
  esp_zb_lock_release();
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
    publishParam(EP_OFFSET, gOffset);
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
    publishParam(EP_SCALE, gScale);
    publishWeight();
  }
  if (wInterval) {
    wInterval = false;
    gInterval = clampInterval(vInterval);
    prefs.putUInt("interval", gInterval);
    publishParam(EP_INTERVAL, gInterval);
  }
  if (wOffset) {
    wOffset = false;
    if (offsetValid(vOffset)) {
      gOffset = vOffset;
      prefs.putFloat("offset", gOffset);
    }
    publishParam(EP_OFFSET, gOffset);
    publishWeight();
  }
  if (wScale) {
    wScale = false;
    if (scaleValid(vScale)) {
      gScale = vScale;
      prefs.putFloat("scale", gScale);
    }
    publishParam(EP_SCALE, gScale);
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

void setup() {
  // Le variant XIAO a déjà activé le commutateur RF (GPIO3 = 0) et choisi l'antenne intégrée (GPIO14 = 0).
  digitalWrite(WIFI_ANT_CONFIG, EXTERNAL_ANTENNA ? HIGH : LOW);
  Serial.begin(115200);
  pinMode(PIN_BOOT, INPUT_PULLUP);
  // Un reset brownout n'est pas une mise sous tension volontaire : pas de mode réglage.
  gSetupMode = esp_sleep_get_wakeup_cause() != ESP_SLEEP_WAKEUP_TIMER && esp_reset_reason() != ESP_RST_BROWNOUT;
  loadSettings();
  measure();  // avant la radio : moins de bruit, et le NAU7802 est coupé plus tôt
  uint32_t mv = readBatteryMv();
  if (batteryTooLow(mv)) {
    Serial.printf("Batterie trop faible (%lu mV) : radio non démarrée\n", (unsigned long)mv);
    sleepNow();
  }
  setupEndpoints(mv);
  attachWriteCallbacks();
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
  publishParam(EP_INTERVAL, gInterval);
  publishParam(EP_OFFSET, gOffset);
  publishParam(EP_SCALE, gScale);
  zbWeight.setBatteryVoltage(mv / 100);
  zbWeight.setBatteryPercentage(batteryPercent(mv));
  publishWeight();
  reportAcked(ESP_ZB_ZCL_CLUSTER_ID_POWER_CONFIG);

  gLastMeasureMs = millis();
  gAwakeUntil = millis() + (gSetupMode ? SETUP_MODE_MS : LISTEN_WINDOW_MS);
}

void loop() {
  applyWrites();
  checkFactoryReset();
  if (gSetupMode && millis() - gLastMeasureMs >= SETUP_MEASURE_PERIOD_MS) {
    gLastMeasureMs = millis();
    if (measure()) publishWeight();
  }
  if ((int32_t)(millis() - gAwakeUntil) >= 0) sleepNow();
  delay(10);
}
