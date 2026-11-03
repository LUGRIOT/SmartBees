#!/bin/sh
# Contrôles statiques du sketch : bash tests/check_sketch.sh
# ZigbeeAnalog::setAnalogOutput() rappelle le callback d'écriture : toute republication locale
# serait vue comme une écriture du coordinateur et le nœud ne dormirait plus jamais.
if grep -n "\.setAnalogOutput(" BalanceRuche/*.ino BalanceRuche/*.h; then
  echo "FAIL: setAnalogOutput() interdit, utiliser publishParam()"
  exit 1
fi
echo "OK check_sketch"
