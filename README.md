# yt-dlv

[![CI](https://github.com/nhodges/yt-dlv/actions/workflows/ci.yml/badge.svg)](https://github.com/nhodges/yt-dlv/actions/workflows/ci.yml)

A YouTube downloader rewritten in Rust — a ground-up reimplementation of
[yt-dlp](https://github.com/yt-dlp/yt-dlp)'s architecture, starting with a
working YouTube vertical slice.

The headline design choice: YouTube's player JavaScript (signature / `n`
descrambling) is executed in an **embedded QuickJS engine compiled into the
binary** — no external Deno/Node/Bun runtime required by default — behind a
pluggable `JsRuntime` trait that can also drive an external runtime when the
hardest challenges (BotGuard / PO tokens) demand it.

> **Status: early, honest WIP.** The full pipeline works end-to-end
> (extraction → format selection → download → ffmpeg mux) and is well tested.
> Real downloads currently rely on the direct-URL InnerTube clients; the
> web-client path and downloads from flagged IPs are gated by YouTube's
> PO-token / BotGuard defenses — see [Status](#status). This is a learning /
> architecture project, not a drop-in yt-dlp replacement (yet).

## Requirements

- **Rust** (stable) and a **C compiler** (`cc`/`clang`) + `make` — the embedded
  QuickJS and SQLite dependencies build from source.
- **ffmpeg** on `PATH` — only needed to merge separate video+audio streams.

## Build & install

```sh
cargo build --release          # binary at ./target/release/yt-dlv
cargo install --path crates/ytdlv-cli   # or install into ~/.cargo/bin
```

## Workspace layout

| Crate | Responsibility |
|-------|----------------|
| `ytdlv-core` | The info-dict contract, the `-f` format-selection language, output-filename templating, and the shared HTTP client. |
| `ytdlv-jsruntime` | The `JsRuntime` trait + embedded **QuickJS** backend (default) + external-runtime backend (Deno/Node/Bun/QuickJS) scaffold. |
| `ytdlv-extractor` | The `Extractor` trait + registry: the **YouTube** extractor (InnerTube clients, `streamingData` parsing, playlists, `base.js` sig/`n` solving) and a **generic** catch-all (direct media + `og:video` scraping). |
| `ytdlv-download` | Download engine: ranged/resumable HTTP and a native **HLS (m3u8)** segment downloader, with progress. |
| `ytdlv-cli` | The `yt-dlv` binary: the orchestrator (yt-dlp's `YoutubeDL` equivalent), `-F` listing, and ffmpeg muxing. |

## Usage

```sh
yt-dlv -F https://www.youtube.com/watch?v=ID            # list formats
yt-dlv -f 'bv*+ba/b' https://youtu.be/ID                # default: best video+audio, merged
yt-dlv -f 'bv*[height<=720]' -o '%(title)s.%(ext)s' URL # filtered selection + template
yt-dlv --player-client ios -j URL                       # dump info JSON via a chosen client
yt-dlv --js-runtime deno URL                            # use an external JS runtime instead of embedded QuickJS

# Authentication (reduces bot-flagging; can unlock the web client):
yt-dlv --cookies cookies.txt URL                        # Netscape cookies.txt
yt-dlv --cookies-from-browser firefox URL               # read straight from a browser profile
yt-dlv --cookies-from-browser 'chrome:Profile 1' URL    # specific Chromium profile

# Networking:
yt-dlv --proxy socks5://user:pass@host:1080 URL         # route via http(s)/socks5 proxy

# Metadata & sidecars:
yt-dlv --print title --print id URL                     # print fields and exit
yt-dlv --skip-download --write-subs --sub-langs en URL  # subtitles only
yt-dlv --write-thumbnail --write-description URL         # thumbnail + description
yt-dlv --flat-playlist --print id 'https://youtube.com/playlist?list=PL...'

# Post-processing & other sources:
yt-dlv -x --audio-format mp3 URL                        # extract audio (ffmpeg)
yt-dlv https://example.com/video.mp4                    # generic: direct media
yt-dlv https://example.com/stream.m3u8                  # generic: HLS
```

Format selection mirrors yt-dlp: `best`/`worst`, `bv*`/`ba*`, `+` to merge,
`/` for fallbacks, `[height<=720]`/`[ext=mp4]`/`[vcodec^=avc1]` filters, and
explicit itags like `137+140`.

## Status

Implemented and tested end-to-end:

- **JS engine**: embedded QuickJS solves signature/`n`-style transforms extracted
  from `base.js` (verified against `base.js`-structured fixtures).
- **Extraction**: InnerTube player API across multiple clients (web/ios/android_vr/tv),
  `streamingData` → typed formats, metadata, thumbnails. `-F` lists real formats
  from live YouTube.
- **Format selection**: `best`/`worst`, `bv*`/`ba*`, `+` merges, `/` fallbacks,
  `[filters]`, explicit ids.
- **Download engine**: ranged HTTP with resume-from-partial (hermetic local-server test),
  plus a native **HLS (m3u8)** downloader.
- **Muxing & audio**: ffmpeg stream-copy of video+audio (mkv fallback) and
  `-x`/`--extract-audio` (mp3/m4a/opus/…).
- **Playlists**: `/playlist?list=` extraction + per-entry re-extraction, `--flat-playlist`.
- **Subtitles**: `--list-subs`, `--write-subs`, `--write-auto-subs`, `--sub-langs`.
- **Sidecars/metadata**: `--print`, `--skip-download`, `--write-info-json`,
  `--write-thumbnail`, `--write-description`.
- **Generic extractor**: downloads direct media URLs and HLS streams from arbitrary
  sites (verified end-to-end — no YouTube PO-token wall).
- **Auth & networking**: Netscape `--cookies`, `--cookies-from-browser`
  (Firefox + Chromium-family decryption), and http/https/`socks5` proxies.

Verified on a residential macOS host: `-f 'bv*+ba/b'` downloads adaptive
av1 + m4a from the `ios` client and muxes a playable `.mp4`.

Known frontier (see issues):

- **PO tokens / BotGuard**: YouTube now enforces proof-of-origin attestation, and
  blocks datacenter IPs outright (`LOGIN_REQUIRED: Sign in to confirm you're not a
  bot`, and `403` from googlevideo even with valid signatures). This is the same
  wall yt-dlp hits; it requires the attestation work tracked in the PO-token issue.
- **Modern `base.js` challenge solving**: current players obfuscate the transforms
  beyond what regex extraction handles reliably (e.g. `split(globalVar)` instead of
  `split("")`). yt-dlp itself has moved to running the whole player via a JS bundle
  in a runtime (`yt-dlp-ejs`); the `JsRuntime` abstraction here is built for the same
  approach.

## Development

```sh
cargo test --workspace      # unit + integration tests (no network required)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

CI runs fmt, clippy (`-D warnings`), build, and tests on every push.

Architecture and the path forward (EJS-style challenge solving, PO tokens) are
tracked in the issues. The `JsRuntime` trait is the integration point for
running more of the player's own JavaScript.

## Disclaimer

A learning/architecture project, not affiliated with yt-dlp or YouTube. Use it
in accordance with YouTube's Terms of Service and applicable law; you are
responsible for how you use it.

## License

Released into the public domain under the [Unlicense](LICENSE).
