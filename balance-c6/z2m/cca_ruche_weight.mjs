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
        // Le nœud refuse aussi, mais une chaîne plus longue que la place réservée ne doit jamais partir.
        const name = String(value);
        if (!/^[\x20-\x7e]{1,16}$/.test(name)) throw new Error('hive_name : 1 à 16 caractères ASCII imprimables');
        const ep = meta.device.getEndpoint(1);
        await ep.write('genBasic', {locationDesc: name});
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
