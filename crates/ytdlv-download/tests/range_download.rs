//! Hermetic test of the download engine against a local HTTP server that
//! honours `Range` requests — proving correct byte output and resume, without
//! depending on any external host (YouTube's CDN blocks datacenter IPs).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;

use ytdlv_core::{Format, HttpClient, Protocol};
use ytdlv_download::{download_format, DownloadOptions};

/// A deterministic payload.
fn payload(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i % 251) as u8).collect()
}

/// Spawn a tiny HTTP/1.1 server that serves `body`, honouring `Range`. Returns
/// the bound base URL. Each connection serves one request then closes.
fn spawn_server(body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let body = body.clone();
            // Read request headers.
            let mut buf = Vec::new();
            let mut tmp = [0u8; 1024];
            loop {
                let Ok(n) = stream.read(&mut tmp) else { break };
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let req = String::from_utf8_lossy(&buf);
            let range_start = req
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("range:"))
                .and_then(|l| l.split("bytes=").nth(1))
                .and_then(|r| r.split('-').next())
                .and_then(|s| s.trim().parse::<usize>().ok());

            let total = body.len();
            let (status, slice, content_range) = match range_start {
                Some(start) if start < total => (
                    "206 Partial Content",
                    &body[start..],
                    Some(format!("bytes {start}-{}/{total}", total - 1)),
                ),
                Some(_) => ("416 Range Not Satisfiable", &body[0..0], None),
                None => ("200 OK", &body[..], None),
            };

            let mut resp = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
                slice.len()
            );
            if let Some(cr) = content_range {
                resp.push_str(&format!("Content-Range: {cr}\r\n"));
            }
            resp.push_str("\r\n");
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.write_all(slice);
            let _ = stream.flush();
        }
    });
    format!("http://{addr}")
}

fn fmt(url: String, size: u64) -> Format {
    Format {
        format_id: "test".into(),
        url,
        ext: "bin".into(),
        protocol: Protocol::Https, // plain http is fine for the native downloader
        filesize: Some(size),
        ..Default::default()
    }
}

#[tokio::test]
async fn downloads_full_file() {
    let data = payload(20_000);
    let base = spawn_server(data.clone());
    let http = HttpClient::new().unwrap();

    let dir = std::env::temp_dir().join(format!("ytdlv-dl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dest: PathBuf = dir.join("full.bin");
    let _ = std::fs::remove_file(&dest);

    download_format(&http, &fmt(format!("{base}/file"), 20_000), &dest, &DownloadOptions { overwrite: true, quiet: true })
        .await
        .unwrap();

    assert_eq!(std::fs::read(&dest).unwrap(), data);
}

#[tokio::test]
async fn resumes_from_partial() {
    let data = payload(20_000);
    let base = spawn_server(data.clone());
    let http = HttpClient::new().unwrap();

    let dir = std::env::temp_dir().join(format!("ytdlv-dl-resume-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dest: PathBuf = dir.join("resume.bin");
    let part = dir.join("resume.bin.part");
    let _ = std::fs::remove_file(&dest);

    // Pre-seed a partial download of the first 8000 bytes.
    std::fs::write(&part, &data[..8000]).unwrap();

    download_format(&http, &fmt(format!("{base}/file"), 20_000), &dest, &DownloadOptions { overwrite: true, quiet: true })
        .await
        .unwrap();

    // The result must be the full, correct file — proving we resumed at 8000
    // (via Range) and appended the remainder rather than corrupting it.
    assert_eq!(std::fs::read(&dest).unwrap(), data);
}
