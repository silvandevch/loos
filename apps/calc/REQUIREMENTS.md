# LoOS Calculator requirements

- Origin: new LoOS app; no vendor source or binary.
- Target: x86-64 PE GUI app, built as `LoCalc.exe` by the Linux MinGW-w64 preset.
- APIs: `user32` and `gdi32` for the window, input, controls, and drawing.
- Behavior: local arithmetic only; no file, registry, network, update, or account access.
- Compatibility profile: Windows 10 manifest; per-monitor DPI and long-path flags
  are app metadata and do not add network or filesystem behavior.
