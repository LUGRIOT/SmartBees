#pragma once
// Broches et constantes. Les réglages modifiables à distance sont dans scale_math.h et en NVS.
#include <stdint.h>

// Broches XIAO ESP32-C6 (NAU7802 sur l'I2C par défaut : SDA D4/GPIO22, SCL D5/GPIO23)
#define PIN_BATTERY A0       // GPIO0 : point milieu du pont 200 k / 200 k
#define PIN_BOOT    BOOT_PIN // GPIO9 : appui long = factory reset Zigbee

// Antenne : false = céramique intégrée, true = antenne externe sur le connecteur U.FL (GPIO14)
constexpr bool EXTERNAL_ANTENNA = true;

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
