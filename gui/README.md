# Codex-Shim GUI

This directory contains a lightweight Tauri v2 desktop shell for `codex-shim`.

## Structure

- `src-tauri/`: Rust desktop backend and Tauri app manifest
- `ui/`: static HTML/CSS/JS frontend with no Node build step

The main window stays focused on:

- runtime status
- token usage curve
- live logs

Configuration lives behind the gear entry point and includes:

- raw shim YAML editing
- model catalog preview
- Codex TOML base editor plus inline merged diff

## Local Development

You do not need Node for the current frontend.

Typical commands:

```bash
cargo check --manifest-path gui/src-tauri/Cargo.toml
cargo run --manifest-path gui/src-tauri/Cargo.toml
```

## Platform Notes

### Linux

Tauri's GTK/WebKit backend requires system packages such as:

- `webkit2gtk`
- `javascriptcoregtk`
- `gtk3`
- `gdk-pixbuf`
- `pango`
- `atk`

If `cargo check` fails in `*-sys` crates with missing `pkg-config` entries,
install those desktop libraries first.

### macOS / Windows

The app is intended to be built primarily on native macOS and Windows hosts.
Those are also the primary GUI targets for `codex-shim`.
