# Command fixtures

`commands.json` holds the exact text written for a fixed list of commands, so that
the two codecs that build frames agree byte for byte:

- the Rust crate, `packages/scent-core` (tested in `tests/commands.rs`);
- the Swift package, `apps/ios/Packages/ScentProtocol` (tested in
  `Tests/ScentProtocolTests/GoldenCommandTests.swift`).

Each entry is one command, named by `name`, with four strings:

```json
{"name": "...", "ble_frame": "<Bluetooth frame, ending in -|>",
 "mqtt_body": "<MQTT message body>", "redacted_bluetooth": "<log text, Bluetooth>",
 "redacted_wifi": "<log text, WiFi>"}
```

Both tests build the same list of commands in the same order. The file was written by
the Rust crate, not by hand. After a deliberate change to what a command writes,
regenerate it with `SCENT_BLESS=1 cargo test -p scent-core --test commands`, then update
the Swift list to match.

The WiFi and broker values are fakes (`ExampleWiFi`, `broker.example.test`,
`example-user`, `example-password`). The time zone cases at the end check how a time
zone offset is written, including values no diffuser would be sent.
