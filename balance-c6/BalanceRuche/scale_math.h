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

constexpr uint32_t BATTERY_CUTOFF_MV = 3300;  // 0 % : la radio ferait chuter la tension (reset brownout)
constexpr uint32_t BATTERY_ABSENT_MV = 1000;  // en dessous : pas de batterie mesurée (USB au banc)

inline bool batteryTooLow(uint32_t mv) {
  return mv >= BATTERY_ABSENT_MV && mv < BATTERY_CUTOFF_MV;
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
