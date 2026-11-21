# Third-party components

The panel's visual design was inspired by **dimap's [yapanel](https://github.com/dimap/yapanel-web)** (MIT, copyright 2026 dimap). The attribution remains in `jarvisd/ui/src/app.css`; its license is preserved in `LICENSES/yapanel-LICENSE`. See [CREDITS.md](CREDITS.md) for the authors and technical references used throughout the project.

- Lucide icons — ISC, https://github.com/lucide-icons/lucide; license preserved in `LICENSES/Lucide-LICENSE`.

- Manrope font — SIL Open Font License 1.1; upstream distribution https://github.com/google/fonts/tree/main/ofl/manrope.
- openWakeWord — Apache-2.0, https://github.com/dscripka/openWakeWord. `models/oww.bin` is a converted feature extractor; its upstream feature models originate from Google's speech embedding work. It is not a bundled openWakeWord wake-word classifier. The custom Jarvis classifier weights are `models/wake.bin` and `models/wake_oww.bin`; raw training recordings are excluded.
- YouTube player processing uses [yt-dlp/ejs](https://github.com/yt-dlp/ejs) at commit
  d48c6303a5a6455a42ef9e0b445149cadea51279 (Unlicense), Meriyah 6.1.4 (ISC)
  and Astring 1.9.0 (MIT). Their notices are in [LICENSES](LICENSES/).
  The bundled JavaScript uses the URL helper and browser shims from EJS.
  To regenerate it, copy `jarvis/modules/youtube/src/preprocess.ts` into the root
  of that EJS checkout as `jarvis-entry.ts`, run `npm ci`, then run the following
  command in the checkout and copy the output back into the module.

  `npx esbuild jarvis-entry.ts --bundle --minify --format=iife --global-name=JarvisEjs --target=es2020 --outfile=ejs.js`

The standard public OAuth identifiers used by upstream clients identify applications; they are not the author's account tokens or personal API keys. Each user must authenticate separately.

Rust dependency versions and sources are pinned in `jarvis/Cargo.lock`.
The optional Rust ChatGPT module bundles Opus through `libopus_sys` (BSD-style;
full notice in `LICENSES/Opus-COPYING`) and uses webrtc-rs (MIT OR Apache-2.0).
The optional YouTube module also uses Opus and runs the player URL helper in
QuickJS through rquickjs (MIT). The QuickJS notice is in the license directory.
The optional Lua runtime uses Lua 5.4 through mlua (MIT).
The Rust VPN module includes separate REALITY attribution and TLS notices in
`jarvis/modules/vpn/THIRD-PARTY.md` and `RUSTLS-LICENSE-MIT.txt`.
Its REALITY TLS fork is vendored with upstream licenses in `jarvis/vendor/reality-tls`;
the Chrome 133 profile follows uTLS (BSD notice in `jarvis/modules/vpn/UTLS-LICENSE.txt`)
and uses RustCrypto ML-KEM (MIT OR Apache-2.0).

License texts for bundled fonts, model components and bundled dependencies are collected under `LICENSES/`. Release archives include these notices alongside the binaries.
