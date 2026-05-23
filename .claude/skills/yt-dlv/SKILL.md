---
name: yt-dlv
description: >-
  Download or inspect a YouTube video using this repo's `yt-dlv` tool (the Rust
  yt-dlp rewrite). Use when the user wants to download a YouTube video/audio,
  list a video's available formats, dump its metadata as JSON, or test that
  yt-dlv works end-to-end. Handles building the binary, format selection,
  ffmpeg muxing, choosing InnerTube clients, and the proxy workaround for the
  datacenter-IP bot block.
---

# Operating yt-dlv

`yt-dlv` is the binary built from this workspace. It downloads YouTube videos by
querying the InnerTube API, solving `base.js` signature/`n` challenges in an
embedded QuickJS engine, downloading via ranged HTTP, and muxing with ffmpeg.

## 1. Prerequisites (check first)

- **Rust toolchain** (`cargo`). The QuickJS dependency also needs a **C compiler**
  (`cc`/`clang`) and `make`.
- **ffmpeg** on PATH — required only to merge separate video+audio streams
  (`-f bv*+ba`). Single muxed formats (`-f 18`, `-f b`) don't need it.

If a tool is missing, tell the user what to install rather than guessing.

## 2. Build

Prefer a release build for real downloads (much faster I/O):

```sh
cargo build --release        # binary at ./target/release/yt-dlv
```

Use `./target/debug/yt-dlv` if a debug build already exists and speed doesn't matter.
Refer to the binary as `$YTDLV` below.

## 3. Common operations

```sh
# List available formats (no download) — best first sanity check
$YTDLV -F "<youtube-url>"

# Default: best video+audio, merged via ffmpeg
$YTDLV -f "bv*+ba/b" -o "%(title)s [%(id)s].%(ext)s" "<url>"

# Single muxed file (no ffmpeg needed)
$YTDLV -f 18 -o "out.%(ext)s" "<url>"

# Cap resolution / filter
$YTDLV -f "bv*[height<=720]+ba/b" "<url>"

# Inspect metadata as JSON (no download)
$YTDLV -j "<url>"

# Verbose logs when something fails
$YTDLV -v -F "<url>"
```

Format-selection syntax mirrors yt-dlp: `best`/`worst`, `bv*`/`ba*`, `+` to merge,
`/` for fallbacks, `[height<=720]`/`[ext=mp4]`/`[vcodec^=avc1]` filters, explicit
itags like `137+140`.

## 4. Choosing an InnerTube client

`yt-dlv` queries several clients and merges formats. If extraction is thin or a
download 403s, force a specific client:

```sh
$YTDLV --player-client ios -F "<url>"          # ios/android_vr often give direct URLs
$YTDLV --player-client web "<url>"             # web needs working sig/nsig solving
```

Order matters; the flag is repeatable for priority: `--player-client ios --player-client web`.

## 5. When downloads fail — diagnose, don't flail

- **`403 Forbidden` from `*.googlevideo.com`**, or `LOGIN_REQUIRED: Sign in to
  confirm you're not a bot`: YouTube is blocking the source IP (common on
  datacenter/cloud IPs) and/or requiring a PO token. To confirm it's server-side,
  extract a URL with `$YTDLV -j --player-client ios "<url>"`, pull a format `url`,
  and `curl -L` it — if curl also 403s, it's not our bug. **Workaround:** route
  through a residential/SOCKS proxy:

  ```sh
  $YTDLV --proxy socks5://user:pass@host:1080 -f 18 "<url>"
  ```

- **`could not locate signature function in base.js`**: the regex solver doesn't
  match the current player; `web`-client formats won't resolve. Use
  `--player-client ios` (direct URLs). This is known and tracked in issue #1
  (EJS-style challenge solving).

- **`ffmpeg not found`**: install ffmpeg, or pick a single muxed format (`-f 18` / `-f b`).

## 6. Verifying a download

Confirm the output is real media, not an HTML/error page:

```sh
file "<output>"            # should report MP4/WebM/Matroska, not HTML/empty
ffprobe "<output>"         # shows streams/duration if ffprobe is available
```

Report the resolved file path and what `file`/`ffprobe` say.

## 7. Reporting back

State plainly which step worked and which didn't, and whether a failure was our
code or YouTube's anti-bot defenses (with the evidence — e.g. the curl result).
Don't modify source unless you've identified a genuine bug.
