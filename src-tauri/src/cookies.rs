//! Import X (Twitter) auth cookies from the user's installed browsers, so the
//! scraper can act as their real logged-in account without them pasting tokens.
//!
//! * Chromium family (Chrome / Edge / Brave): cookies live in a SQLite DB with
//!   AES-256-GCM encrypted values. The AES key is stored in `Local State`,
//!   itself wrapped with Windows DPAPI. Very recent Chrome (v20 "app-bound"
//!   encryption) can't be unwrapped this way — we surface a clear error and let
//!   the user fall back to a manual token or a different browser.
//! * Firefox: cookies are stored unencrypted in `cookies.sqlite`.
//!
//! We only ever read `auth_token` and `ct0` for x.com / twitter.com.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum CookieError {
    #[error("{0} is not installed (no profile folder found)")]
    NotInstalled(String),
    #[error("no X/Twitter login found in {0} — make sure you're logged into x.com there")]
    NoCookies(String),
    #[error("couldn't read {0}'s cookie database: {1}")]
    Db(String, String),
    #[error("couldn't decrypt {0}'s cookies — recent Chrome app-bound encryption isn't supported; try another browser or paste a token manually")]
    Decrypt(String),
    #[error("unknown browser: {0}")]
    Unknown(String),
    #[error("io error: {0}")]
    Io(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowserKind {
    Chromium,
    Firefox,
}

struct BrowserDef {
    id: &'static str,
    label: &'static str,
    kind: BrowserKind,
    /// Env var holding the base dir ("LOCALAPPDATA" or "APPDATA").
    base_env: &'static str,
    /// Path under that base dir to the browser's data root.
    rel_path: &'static str,
}

const BROWSERS: &[BrowserDef] = &[
    BrowserDef { id: "chrome", label: "Google Chrome", kind: BrowserKind::Chromium, base_env: "LOCALAPPDATA", rel_path: r"Google\Chrome\User Data" },
    BrowserDef { id: "edge", label: "Microsoft Edge", kind: BrowserKind::Chromium, base_env: "LOCALAPPDATA", rel_path: r"Microsoft\Edge\User Data" },
    BrowserDef { id: "brave", label: "Brave", kind: BrowserKind::Chromium, base_env: "LOCALAPPDATA", rel_path: r"BraveSoftware\Brave-Browser\User Data" },
    BrowserDef { id: "firefox", label: "Firefox", kind: BrowserKind::Firefox, base_env: "APPDATA", rel_path: r"Mozilla\Firefox" },
];

/// A browser the user has installed, for the frontend account picker.
#[derive(Debug, Serialize)]
pub struct BrowserInfo {
    pub id: String,
    pub label: String,
}

/// Credentials lifted from a browser, ready to hand to the scraper.
pub struct ImportedSession {
    pub auth_token: String,
    pub ct0: Option<String>,
}

impl ImportedSession {
    /// Format as the `auth_token=…; ct0=…` cookie string the scraper parses.
    pub fn as_cookie_string(&self) -> String {
        match &self.ct0 {
            Some(ct0) => format!("auth_token={}; ct0={}", self.auth_token, ct0),
            None => format!("auth_token={}", self.auth_token),
        }
    }
}

fn def(id: &str) -> Option<&'static BrowserDef> {
    BROWSERS.iter().find(|b| b.id == id)
}

fn data_root(d: &BrowserDef) -> Option<PathBuf> {
    let base = std::env::var(d.base_env).ok()?;
    let p = Path::new(&base).join(d.rel_path);
    p.is_dir().then_some(p)
}

/// Browsers that are actually installed (have a data folder present).
pub fn list_installed() -> Vec<BrowserInfo> {
    BROWSERS
        .iter()
        .filter(|d| data_root(d).is_some())
        .map(|d| BrowserInfo { id: d.id.to_string(), label: d.label.to_string() })
        .collect()
}

/// Import X auth cookies from the given browser id.
pub fn import(browser_id: &str) -> Result<ImportedSession, CookieError> {
    let d = def(browser_id).ok_or_else(|| CookieError::Unknown(browser_id.to_string()))?;
    let root = data_root(d).ok_or_else(|| CookieError::NotInstalled(d.label.to_string()))?;
    match d.kind {
        BrowserKind::Chromium => import_chromium(d, &root),
        BrowserKind::Firefox => import_firefox(d, &root),
    }
}

// --- Chromium ---------------------------------------------------------------

fn import_chromium(d: &BrowserDef, root: &Path) -> Result<ImportedSession, CookieError> {
    let key = chromium_master_key(d, root)?;

    let mut fallback: Option<ImportedSession> = None;
    let mut had_decrypt_error = false;

    for profile in chromium_profiles(root) {
        let db = profile.join("Network").join("Cookies");
        let db = if db.is_file() { db } else { profile.join("Cookies") };
        if !db.is_file() {
            continue;
        }
        let rows = read_db(d, &db, "SELECT name, encrypted_value FROM cookies WHERE name IN ('auth_token','ct0') AND (host_key LIKE '%x.com' OR host_key LIKE '%twitter.com')")?;

        // Keep auth_token + ct0 paired per profile — mixing them across profiles
        // would marry one account's token to another account's CSRF value.
        let mut auth_token = None;
        let mut ct0 = None;
        for (name, blob) in rows {
            let val = match decrypt_chromium_value(d, &blob, &key) {
                Ok(v) if !v.is_empty() => v,
                Ok(_) => continue,
                // One undecryptable row (e.g. a v20 app-bound value) shouldn't
                // sink a sibling row that decrypts fine.
                Err(_) => {
                    had_decrypt_error = true;
                    continue;
                }
            };
            match name.as_str() {
                "auth_token" => auth_token = Some(val),
                "ct0" => ct0 = Some(val),
                _ => {}
            }
        }

        match (auth_token, ct0) {
            // A complete session — prefer it and stop.
            (Some(auth_token), Some(ct0)) => {
                return Ok(ImportedSession { auth_token, ct0: Some(ct0) })
            }
            // auth_token only: remember the first as a fallback, but keep looking
            // for a profile that also has the matching ct0.
            (Some(auth_token), None) if fallback.is_none() => {
                fallback = Some(ImportedSession { auth_token, ct0: None })
            }
            _ => {}
        }
    }

    fallback.ok_or_else(|| {
        if had_decrypt_error {
            CookieError::Decrypt(d.label.to_string())
        } else {
            CookieError::NoCookies(d.label.to_string())
        }
    })
}

fn chromium_profiles(root: &Path) -> Vec<PathBuf> {
    let mut profiles = Vec::new();
    let default = root.join("Default");
    if default.is_dir() {
        profiles.push(default);
    }
    if let Ok(entries) = fs::read_dir(root) {
        for e in entries.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("Profile ") && e.path().is_dir() {
                profiles.push(e.path());
            }
        }
    }
    profiles
}

/// Read `Local State`, base64-decode `os_crypt.encrypted_key`, strip the "DPAPI"
/// prefix and DPAPI-unwrap it into the 32-byte AES key.
fn chromium_master_key(d: &BrowserDef, root: &Path) -> Result<Vec<u8>, CookieError> {
    use base64::{engine::general_purpose::STANDARD, Engine};

    let local_state = root.join("Local State");
    let text = fs::read_to_string(&local_state).map_err(|e| CookieError::Io(e.to_string()))?;
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| CookieError::Io(e.to_string()))?;
    let key_b64 = json
        .pointer("/os_crypt/encrypted_key")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CookieError::Decrypt(d.label.to_string()))?;
    let mut key = STANDARD
        .decode(key_b64)
        .map_err(|_| CookieError::Decrypt(d.label.to_string()))?;
    if key.starts_with(b"DPAPI") {
        key.drain(0..5);
    }
    dpapi_decrypt(&key).map_err(|_| CookieError::Decrypt(d.label.to_string()))
}

fn decrypt_chromium_value(d: &BrowserDef, enc: &[u8], key: &[u8]) -> Result<String, CookieError> {
    use aes_gcm::aead::{generic_array::GenericArray, Aead};
    use aes_gcm::{Aes256Gcm, KeyInit};

    // v20 = app-bound encryption (recent Chrome): key isn't the Local State one.
    if enc.starts_with(b"v20") {
        return Err(CookieError::Decrypt(d.label.to_string()));
    }
    // v10 / v11 = AES-256-GCM: [3-byte tag][12-byte nonce][ciphertext+16-byte gcm tag].
    if enc.starts_with(b"v10") || enc.starts_with(b"v11") {
        if enc.len() < 3 + 12 + 16 {
            return Err(CookieError::Decrypt(d.label.to_string()));
        }
        // GenericArray::from_slice panics on a wrong length, so guard the key.
        if key.len() != 32 {
            return Err(CookieError::Decrypt(d.label.to_string()));
        }
        let cipher = Aes256Gcm::new(GenericArray::from_slice(key));
        let nonce = GenericArray::from_slice(&enc[3..15]);
        let plain = cipher
            .decrypt(nonce, &enc[15..])
            .map_err(|_| CookieError::Decrypt(d.label.to_string()))?;
        return Ok(String::from_utf8_lossy(&plain).into_owned());
    }
    // Legacy: whole value is DPAPI-encrypted.
    let plain = dpapi_decrypt(enc).map_err(|_| CookieError::Decrypt(d.label.to_string()))?;
    Ok(String::from_utf8_lossy(&plain).into_owned())
}

// --- Firefox ----------------------------------------------------------------

fn import_firefox(d: &BrowserDef, root: &Path) -> Result<ImportedSession, CookieError> {
    let profiles = root.join("Profiles");
    let mut auth_token = None;
    let mut ct0 = None;

    if let Ok(entries) = fs::read_dir(&profiles) {
        for e in entries.flatten() {
            let db = e.path().join("cookies.sqlite");
            if !db.is_file() {
                continue;
            }
            let rows = read_db(d, &db, "SELECT name, value FROM moz_cookies WHERE name IN ('auth_token','ct0') AND (host LIKE '%x.com' OR host LIKE '%twitter.com')")?;
            for (name, blob) in rows {
                let val = String::from_utf8_lossy(&blob).into_owned();
                match name.as_str() {
                    "auth_token" if !val.is_empty() => auth_token = Some(val),
                    "ct0" if !val.is_empty() => ct0 = Some(val),
                    _ => {}
                }
            }
            if auth_token.is_some() {
                break;
            }
        }
    }

    match auth_token {
        Some(auth_token) => Ok(ImportedSession { auth_token, ct0 }),
        None => Err(CookieError::NoCookies(d.label.to_string())),
    }
}

// --- SQLite helper ----------------------------------------------------------

/// Copy the (possibly browser-locked) DB to a temp file and run `query`,
/// returning `(name, value-bytes)` rows. Firefox `value` comes back as text
/// bytes; Chromium `encrypted_value` as the raw blob.
fn read_db(d: &BrowserDef, db: &Path, query: &str) -> Result<Vec<(String, Vec<u8>)>, CookieError> {
    use rusqlite::{Connection, OpenFlags};

    let label = d.label.to_string();
    let copy = TempCopy::new(db).map_err(|e| CookieError::Db(label.clone(), e))?;

    let conn = Connection::open_with_flags(
        &copy.db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| CookieError::Db(label.clone(), e.to_string()))?;

    let mut stmt = conn
        .prepare(query)
        .map_err(|e| CookieError::Db(label.clone(), e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            let name: String = row.get(0)?;
            let value: Vec<u8> = row.get(1)?;
            Ok((name, value))
        })
        .map_err(|e| CookieError::Db(label.clone(), e.to_string()))?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| CookieError::Db(label.clone(), e.to_string()))?);
    }
    Ok(out)
}

/// A private copy of a (possibly browser-locked) cookie DB and its
/// `-wal`/`-journal` siblings. The copies contain live auth cookies, so they go
/// in a uniquely-named, owner-only (mode 0700) temp subdirectory — on the shared
/// temp dir other local users could otherwise read them. Removed on drop.
struct TempCopy {
    dir: PathBuf,
    db: PathBuf,
}

impl TempCopy {
    fn new(src: &Path) -> Result<Self, String> {
        let dir = unique_private_dir().map_err(|e| e.to_string())?;
        let db = dir.join("Cookies.sqlite");
        copy_private(src, &db)?;
        for suffix in ["-wal", "-journal"] {
            let mut s = src.as_os_str().to_owned();
            s.push(suffix);
            let s = PathBuf::from(s);
            if s.is_file() {
                let mut d = db.as_os_str().to_owned();
                d.push(suffix);
                let _ = copy_private(&s, &PathBuf::from(d));
            }
        }
        Ok(Self { dir, db })
    }
}

impl Drop for TempCopy {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Create a fresh temp subdirectory only the current user can enter. Uses
/// non-recursive create so a pre-existing (squatted) dir is rejected.
fn unique_private_dir() -> std::io::Result<PathBuf> {
    let name = format!(
        "tmd-cookies-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let dir = std::env::temp_dir().join(name);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(&dir)?;
    }
    #[cfg(not(unix))]
    {
        // %TEMP% is already per-user on Windows; the unique non-recursive
        // create still rejects a squatted directory.
        fs::DirBuilder::new().create(&dir)?;
    }
    Ok(dir)
}

/// Copy a file, then tighten it to owner-only (0600) on Unix.
fn copy_private(src: &Path, dest: &Path) -> Result<(), String> {
    fs::copy(src, dest).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dest, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

// --- DPAPI ------------------------------------------------------------------

#[cfg(windows)]
fn dpapi_decrypt(data: &[u8]) -> Result<Vec<u8>, String> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};

    let in_blob = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut out_blob = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    unsafe {
        CryptUnprotectData(&in_blob, None, None, None, None, 0, &mut out_blob)
            .map_err(|e| e.to_string())?;

        let result = std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out_blob.pbData as *mut _)));
        Ok(result)
    }
}

#[cfg(not(windows))]
fn dpapi_decrypt(_data: &[u8]) -> Result<Vec<u8>, String> {
    Err("DPAPI is only available on Windows".to_string())
}