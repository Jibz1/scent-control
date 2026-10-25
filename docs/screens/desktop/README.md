# Desktop screens

Settings in light and dark mode, rendered from the desktop app’s sample data.
The broker address, username, route ID, and build identifier are examples.

To regenerate from the repository root:

```sh
cargo test -p scent-desktop render_screens -- --ignored
cp target/snapshots/settings_light.png docs/screens/desktop/
cp target/snapshots/settings_dark.png docs/screens/desktop/
```

The renderer does not connect to a broker or read saved account settings.
