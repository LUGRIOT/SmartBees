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
