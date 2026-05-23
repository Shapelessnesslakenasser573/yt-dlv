# yt-dlv

A YouTube downloader rewritten in Rust — a ground-up reimplementation of
[yt-dlp](https://github.com/yt-dlp/yt-dlp)'s architecture, starting with a
working YouTube vertical slice.

The headline design choice: YouTube's player JavaScript (signature / `n`
descrambling) is executed in an **embedded QuickJS engine compiled into the
binary** — no external Deno/Node/Bun runtime required by default — behind a
pluggable `JsRuntime` trait that can also drive an external runtime when the
hardest challenges (BotGuard / PO tokens) demand it.

## Workspace layout

| Crate | Responsibility |
|-------|----------------|
| `ytdlv-core` | The info-dict contract, the `-f` format-selection language, output-filename templating, and the shared HTTP client. |
| `ytdlv-jsruntime` | The `JsRuntime` trait + embedded **QuickJS** backend (default) + external-runtime backend (Deno/Node/Bun/QuickJS) scaffold. |
| `ytdlv-extractor` | The `Extractor` trait + registry, and the **YouTube** extractor: InnerTube clients, `streamingData` parsing, and `base.js` signature/`n` solving. |
| `ytdlv-download` | Ranged, resumable HTTP download engine with progress. |
| `ytdlv-cli` | The `yt-dlv` binary: the orchestrator (yt-dlp's `YoutubeDL` equivalent), `-F` listing, and ffmpeg muxing. |

## Usage

```sh
yt-dlv -F https://www.youtube.com/watch?v=ID            # list formats
yt-dlv -f 'bv*+ba/b' https://youtu.be/ID                # default: best video+audio, merged
yt-dlv -f 'bv*[height<=720]' -o '%(title)s.%(ext)s' URL # filtered selection + template
yt-dlv --player-client ios -j URL                       # dump info JSON via a chosen client
yt-dlv --js-runtime deno URL                            # use an external JS runtime instead of embedded QuickJS
```

## Status

Implemented and tested end-to-end:

- **JS engine**: embedded QuickJS solves signature/`n`-style transforms extracted
  from `base.js` (verified against `base.js`-structured fixtures).
- **Extraction**: InnerTube player API across multiple clients (web/ios/android_vr/tv),
  `streamingData` → typed formats, metadata, thumbnails. `-F` lists real formats
  from live YouTube.
- **Format selection**: `best`/`worst`, `bv*`/`ba*`, `+` merges, `/` fallbacks,
  `[filters]`, explicit ids.
- **Download engine**: ranged HTTP with resume-from-partial (hermetic local-server test).
- **Muxing**: ffmpeg stream-copy of video+audio, with mkv fallback.

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

## License

Unlicense.
