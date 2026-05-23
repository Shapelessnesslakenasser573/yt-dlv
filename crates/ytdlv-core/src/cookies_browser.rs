//! Extract cookies straight from a local browser profile — yt-dlp's
//! `--cookies-from-browser`.
//!
//! - **Firefox** stores cookies as plaintext in `cookies.sqlite`; fully supported.
//! - **Chromium family** (Chrome/Chromium/Brave/Edge) encrypts cookie values.
//!   We decrypt the standard `v10`/`v11` scheme (AES-128-CBC, key derived via
//!   PBKDF2-HMAC-SHA1 over a per-OS secret). macOS reads the key from the
//!   Keychain; Linux uses the well-known `peanuts` fallback. Windows DPAPI /
//!   Chrome app-bound encryption are not handled — use `--cookies` there.
//!
//! This is best-effort and tracks Chrome's evolving format; if decryption
//! fails, exporting a `cookies.txt` and using `--cookies` always works.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use reqwest::cookie::Jar;

use crate::cookies::{jar_from_cookies, NetscapeCookie};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Browser {
    Firefox,
    Chrome,
    Chromium,
    Brave,
    Edge,
}

impl Browser {
    fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "firefox" => Some(Self::Firefox),
            "chrome" => Some(Self::Chrome),
            "chromium" => Some(Self::Chromium),
            "brave" => Some(Self::Brave),
            "edge" => Some(Self::Edge),
            _ => None,
        }
    }

    fn is_chromium(self) -> bool {
        !matches!(self, Self::Firefox)
    }

    /// Keychain "Safe Storage" service name (macOS) for Chromium browsers.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn keychain_service(self) -> &'static str {
        match self {
            Self::Chrome => "Chrome Safe Storage",
            Self::Chromium => "Chromium Safe Storage",
            Self::Brave => "Brave Safe Storage",
            Self::Edge => "Microsoft Edge Safe Storage",
            Self::Firefox => "",
        }
    }
}

/// Load cookies for `spec` = `browser[:profile]` into a jar.
pub fn load_from_browser(spec: &str) -> Result<Arc<Jar>> {
    let (name, profile) = match spec.split_once(':') {
        Some((b, p)) => (b, Some(p)),
        None => (spec, None),
    };
    let browser = Browser::parse(name).ok_or_else(|| {
        anyhow!("unknown browser '{name}' (try firefox|chrome|chromium|brave|edge)")
    })?;

    let cookies = if browser.is_chromium() {
        chromium_cookies(browser, profile)?
    } else {
        firefox_cookies(profile)?
    };
    if cookies.is_empty() {
        bail!("no cookies found for {name}");
    }
    tracing::info!("loaded {} cookies from {name}", cookies.len());
    Ok(jar_from_cookies(&cookies))
}

// ---------------------------------------------------------------------------
// SQLite helper
// ---------------------------------------------------------------------------

/// Open a (possibly locked) browser DB by copying it to a temp file first.
fn open_db_copy(db: &Path) -> Result<(rusqlite::Connection, PathBuf)> {
    if !db.exists() {
        bail!("cookie database not found at {}", db.display());
    }
    let tmp = std::env::temp_dir().join(format!(
        "ytdlv-cookies-{}-{}.sqlite",
        std::process::id(),
        rand_suffix()
    ));
    std::fs::copy(db, &tmp).with_context(|| format!("copying {}", db.display()))?;
    let conn = rusqlite::Connection::open(&tmp)
        .with_context(|| format!("opening cookie db copy {}", tmp.display()))?;
    Ok((conn, tmp))
}

fn rand_suffix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Firefox (plaintext)
// ---------------------------------------------------------------------------

fn firefox_cookies(profile: Option<&str>) -> Result<Vec<NetscapeCookie>> {
    let base = firefox_base_dir().ok_or_else(|| anyhow!("could not locate Firefox directory"))?;
    let db = find_firefox_cookie_db(&base, profile)
        .ok_or_else(|| anyhow!("no Firefox cookies.sqlite found under {}", base.display()))?;
    let (conn, tmp) = open_db_copy(&db)?;
    let result = read_moz_cookies(&conn);
    let _ = std::fs::remove_file(&tmp);
    result
}

/// Read Firefox's `moz_cookies` table into our cookie type.
fn read_moz_cookies(conn: &rusqlite::Connection) -> Result<Vec<NetscapeCookie>> {
    let mut stmt = conn.prepare("SELECT host, name, value, path, isSecure FROM moz_cookies")?;
    let rows = stmt.query_map([], |row| {
        let domain: String = row.get(0)?;
        Ok(NetscapeCookie {
            include_subdomains: domain.starts_with('.'),
            domain,
            path: row.get(3)?,
            secure: row.get::<_, i64>(4)? != 0,
            expires: None,
            name: row.get(1)?,
            value: row.get(2)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

fn firefox_base_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        Some(dirs::config_dir()?.join("Firefox"))
    } else if cfg!(target_os = "windows") {
        Some(dirs::data_dir()?.join("Mozilla").join("Firefox"))
    } else {
        Some(dirs::home_dir()?.join(".mozilla").join("firefox"))
    }
}

/// Locate a profile's cookies.sqlite, preferring a name match, then a
/// `*.default-release`/`*.default` profile, then any.
fn find_firefox_cookie_db(base: &Path, profile: Option<&str>) -> Option<PathBuf> {
    let mut search_roots = vec![base.to_path_buf()];
    let profiles = base.join("Profiles");
    if profiles.is_dir() {
        search_roots.push(profiles);
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    for root in &search_roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for e in entries.flatten() {
            let db = e.path().join("cookies.sqlite");
            if db.is_file() {
                candidates.push(db);
            }
        }
    }

    let dir_name = |p: &Path| {
        p.parent()
            .and_then(|d| d.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    };

    if let Some(want) = profile {
        if let Some(hit) = candidates.iter().find(|p| dir_name(p).contains(want)) {
            return Some(hit.clone());
        }
    }
    candidates
        .iter()
        .find(|p| dir_name(p).ends_with(".default-release"))
        .or_else(|| {
            candidates
                .iter()
                .find(|p| dir_name(p).ends_with(".default"))
        })
        .or_else(|| candidates.first())
        .cloned()
}

// ---------------------------------------------------------------------------
// Chromium family (encrypted)
// ---------------------------------------------------------------------------

fn chromium_cookies(browser: Browser, profile: Option<&str>) -> Result<Vec<NetscapeCookie>> {
    let base = chromium_base_dir(browser)
        .ok_or_else(|| anyhow!("could not locate browser data directory"))?;
    let profile_dir = base.join(profile.unwrap_or("Default"));
    // Newer Chromium keeps cookies under Network/.
    let db = [
        profile_dir.join("Network").join("Cookies"),
        profile_dir.join("Cookies"),
    ]
    .into_iter()
    .find(|p| p.is_file())
    .ok_or_else(|| anyhow!("no Cookies DB under {}", profile_dir.display()))?;

    let key = chromium_key(browser)?;
    let (conn, tmp) = open_db_copy(&db)?;

    let result = (|| -> Result<Vec<NetscapeCookie>> {
        let mut stmt =
            conn.prepare("SELECT host_key, name, encrypted_value, path, is_secure FROM cookies")?;
        let rows = stmt.query_map([], |row| {
            let host: String = row.get(0)?;
            let name: String = row.get(1)?;
            let enc: Vec<u8> = row.get(2)?;
            let path: String = row.get(3)?;
            let secure: i64 = row.get(4)?;
            Ok((host, name, enc, path, secure))
        })?;

        let mut out = Vec::new();
        for r in rows.flatten() {
            let (host, name, enc, path, secure) = r;
            match decrypt_chromium_value(&enc, &key) {
                Ok(value) => out.push(NetscapeCookie {
                    include_subdomains: host.starts_with('.'),
                    domain: host,
                    path,
                    secure: secure != 0,
                    expires: None,
                    name,
                    value,
                }),
                Err(e) => tracing::debug!("skipping cookie '{name}': {e}"),
            }
        }
        Ok(out)
    })();

    let _ = std::fs::remove_file(&tmp);
    result
}

fn chromium_base_dir(browser: Browser) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let cfg = dirs::config_dir()?;
    let p = if cfg!(target_os = "macos") {
        // dirs::config_dir() == ~/Library/Application Support on macOS.
        match browser {
            Browser::Chrome => cfg.join("Google/Chrome"),
            Browser::Chromium => cfg.join("Chromium"),
            Browser::Brave => cfg.join("BraveSoftware/Brave-Browser"),
            Browser::Edge => cfg.join("Microsoft Edge"),
            Browser::Firefox => return None,
        }
    } else if cfg!(target_os = "windows") {
        let local = dirs::data_local_dir()?;
        match browser {
            Browser::Chrome => local.join("Google/Chrome/User Data"),
            Browser::Chromium => local.join("Chromium/User Data"),
            Browser::Brave => local.join("BraveSoftware/Brave-Browser/User Data"),
            Browser::Edge => local.join("Microsoft/Edge/User Data"),
            Browser::Firefox => return None,
        }
    } else {
        // Linux (~/.config).
        let _ = &home;
        match browser {
            Browser::Chrome => cfg.join("google-chrome"),
            Browser::Chromium => cfg.join("chromium"),
            Browser::Brave => cfg.join("BraveSoftware/Brave-Browser"),
            Browser::Edge => cfg.join("microsoft-edge"),
            Browser::Firefox => return None,
        }
    };
    Some(p)
}

/// Derive the AES key for Chromium cookie decryption.
fn chromium_key(browser: Browser) -> Result<[u8; 16]> {
    let (password, iterations): (Vec<u8>, u32) = if cfg!(target_os = "macos") {
        (macos_keychain_password(browser)?, 1003)
    } else if cfg!(target_os = "windows") {
        bail!(
            "Chromium cookie decryption on Windows (DPAPI / app-bound) is not supported; \
             export cookies and use --cookies instead"
        );
    } else {
        // Linux without a desktop keyring uses this well-known secret.
        (b"peanuts".to_vec(), 1)
    };
    Ok(pbkdf2::pbkdf2_hmac_array::<sha1::Sha1, 16>(
        &password,
        b"saltysalt",
        iterations,
    ))
}

#[cfg(target_os = "macos")]
fn macos_keychain_password(browser: Browser) -> Result<Vec<u8>> {
    let out = std::process::Command::new("security")
        .args(["find-generic-password", "-wa", browser.keychain_service()])
        .output()
        .context("invoking macOS `security` to read the Keychain")?;
    if !out.status.success() {
        bail!(
            "Keychain lookup for '{}' failed (you may be prompted to allow access): {}",
            browser.keychain_service(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .trim()
        .as_bytes()
        .to_vec())
}

#[cfg(not(target_os = "macos"))]
fn macos_keychain_password(_browser: Browser) -> Result<Vec<u8>> {
    unreachable!("only called on macOS")
}

/// Decrypt a Chromium `encrypted_value` (the `v10`/`v11` AES-CBC scheme).
fn decrypt_chromium_value(enc: &[u8], key: &[u8; 16]) -> Result<String> {
    use aes::Aes128;
    use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};

    if enc.len() < 3 || !(enc.starts_with(b"v10") || enc.starts_with(b"v11")) {
        bail!("unsupported cookie encryption (no v10/v11 prefix)");
    }
    let iv = [0x20u8; 16]; // 16 spaces
    let mut buf = enc[3..].to_vec();
    if !buf.len().is_multiple_of(16) || buf.is_empty() {
        bail!("ciphertext not block-aligned");
    }
    let dec = cbc::Decryptor::<Aes128>::new_from_slices(key, &iv)
        .map_err(|e| anyhow!("aes init: {e}"))?
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|e| anyhow!("aes decrypt: {e}"))?;

    // Newer Chrome prepends a 32-byte SHA-256 domain hash to the plaintext.
    if let Ok(s) = std::str::from_utf8(dec) {
        return Ok(s.to_string());
    }
    if dec.len() > 32 {
        if let Ok(s) = std::str::from_utf8(&dec[32..]) {
            return Ok(s.to_string());
        }
    }
    bail!("decrypted cookie value is not valid UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_spec_parsing() {
        assert_eq!(Browser::parse("firefox"), Some(Browser::Firefox));
        assert_eq!(Browser::parse("Chrome"), Some(Browser::Chrome));
        assert_eq!(Browser::parse("nope"), None);
        assert!(Browser::Chrome.is_chromium());
        assert!(!Browser::Firefox.is_chromium());
    }

    #[test]
    fn chromium_decrypt_roundtrip() {
        use aes::Aes128;
        use cbc::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};

        let key = pbkdf2::pbkdf2_hmac_array::<sha1::Sha1, 16>(b"peanuts", b"saltysalt", 1);
        let iv = [0x20u8; 16];
        let plaintext = b"SID=secret-value-123";

        // Encrypt exactly as Chromium does, with the v10 prefix.
        let mut buf = vec![0u8; plaintext.len() + 16];
        buf[..plaintext.len()].copy_from_slice(plaintext);
        let ct = cbc::Encryptor::<Aes128>::new_from_slices(&key, &iv)
            .unwrap()
            .encrypt_padded_mut::<Pkcs7>(&mut buf, plaintext.len())
            .unwrap();
        let mut encrypted = b"v10".to_vec();
        encrypted.extend_from_slice(ct);

        let value = decrypt_chromium_value(&encrypted, &key).unwrap();
        assert_eq!(value, "SID=secret-value-123");
    }

    #[test]
    fn rejects_unencrypted_blob() {
        let key = [0u8; 16];
        assert!(decrypt_chromium_value(b"not-v10-prefixed", &key).is_err());
    }

    #[test]
    fn reads_firefox_moz_cookies_table() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE moz_cookies (host TEXT, name TEXT, value TEXT, path TEXT, isSecure INTEGER);
             INSERT INTO moz_cookies VALUES ('.youtube.com','SID','abc','/',1);
             INSERT INTO moz_cookies VALUES ('www.youtube.com','PREF','x','/',0);",
        )
        .unwrap();

        let cookies = read_moz_cookies(&conn).unwrap();
        assert_eq!(cookies.len(), 2);
        assert_eq!(cookies[0].name, "SID");
        assert!(cookies[0].include_subdomains && cookies[0].secure);
        assert_eq!(cookies[1].name, "PREF");
        assert!(!cookies[1].secure);
    }
}
