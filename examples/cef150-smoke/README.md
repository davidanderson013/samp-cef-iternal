# CEF 150 client smoke test

This fixture checks that the CEF 150 client preserves the behavior of the previous plugin while using the official non-proprietary runtime.

It covers:

- local `sampcef` asset loading with `text/html` responses;
- HTTP fetches;
- browser/renderer JS IPC through `cef.on`, `cef.emit`, and `cef.set_focus`;
- software OSR animation, resizing, keyboard, and mouse input;
- an external browser attached to an SA:MP object texture;
- VP9 + Opus playback;
- expected H.264 + AAC unavailability.

## Assets

Copy these files into `<GTA>/cef/assets/cef150-smoke/`:

- `index.html` from this directory;
- `object.html` from this directory;
- Chromium's `bear-vp9-opus.webm` sample;
- Chromium's `bear-1280x720.mp4` sample renamed to `bear-h264-aac.mp4`.

The media samples are not stored in this repository.

## Server

1. Run the HTTP probe from the repository root:

   ```sh
   node scripts/smoke-http-server.mjs
   ```

2. Compile `mode.pwn` with `server/cef.inc` available to the Pawn compiler.
3. Load the resulting gamemode together with either the SA:MP CEF server plugin or the open.mp component.
4. Start SA:MP through `samp.exe` and connect to the test server.

The HTTP probe listens on `127.0.0.1:18080`. With the default SA:MP port `7777` and the default CEF port offset of `2`, the CEF transport listens on `7779`.

## Expected result

The overlay reports successful JS API discovery, HTTP and IPC round trips, WebM playback, and rejected H.264 playback. Selecting **Show object texture** hides the overlay and displays a cyan/magenta checker with `CEF 150 OBJECT` on the spawned object.
