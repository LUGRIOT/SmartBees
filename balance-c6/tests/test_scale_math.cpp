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

  // Batterie trop faible pour démarrer la radio (évite les resets brownout en boucle)
  assert(batteryTooLow(3299));
  assert(batteryTooLow(1000));
  assert(!batteryTooLow(3300));
  assert(!batteryTooLow(4200));
  assert(!batteryTooLow(999));  // pas de batterie mesurée (alimentation USB au banc)
  assert(!batteryTooLow(0));

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
