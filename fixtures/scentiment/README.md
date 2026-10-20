# Scentiment fixtures

Real messages captured from our diffusers, sanitized, for testing protocol code.
Each file is one message:

```json
{"source": "observed YYYY-MM-DD", "device": "air2" | "hvac", "firmware": "2.30",
 "channel": "ble-notify" | "ble-info" | "mqtt", "note"?: "...", "text": "<exact text>"}
```

Sanitizing rules:
- device IDs become `TESTDEVICE01` (Air 2) and `TESTDEVICE02` (HVAC 2);
- WiFi names become `ExampleWiFi`;
- broker hosts become `broker.example.test`;
- passwords never appear.

Everything else is byte-for-byte what the device sent. For MQTT messages, `text` is
the full message body and `note` names the topic pattern.
