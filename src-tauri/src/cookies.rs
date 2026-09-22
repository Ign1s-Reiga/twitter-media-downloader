//! Import a site's auth cookies from the user's installed browsers, so a
//! scraper can act as the real logged-in account without pasted tokens.
//!
//! * Chromium family (Chrome / Edge / Brave): cookies live in a SQLite DB with
//!   AES-256-GCM encrypted values. The AES key is stored in `Local State`,
//!   itself wrapped with Windows DPAPI. Very recent Chrome (v20 "app-bound"
//!   encryption) can't be unwrapped this way — we surface a clear error and let
//!   the user fall back to a manual token or a different browser.
//! * Firefox: cookies are stored unencrypted in `cookies.sqlite`. Multi-Account
//!   Containers keep separate jars in the same table, told apart by the
//!   `originAttributes` column — each container is offered as its own entry so
//!   a scraping-account container can be picked over the main login.
//!
//! We read every cookie stored for the site's hosts (a `SiteCookies` spec)
//! and nothing else: sites like Instagram treat a session cookie that shows
//! up without the device cookies issued alongside it (`mid`, `datr`, …) as
//! suspicious, so the whole jar travels together.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum CookieError {
    #[error("{0} is not installed (no profile folder found)")]
    NotInstalled(String),
    #[error("no {site} login found in {browser} — make sure you're logged into {site} there")]
    NoCookies { browser: String, site: &'static str },
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
    /// A User-Agent of this browser family, so requests made with its cookies
    /// look like they come from the browser that owns the session.
    user_agent: &'static str,
}

const CHROME_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
const EDGE_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36 Edg/124.0.0.0";
const FIREFOX_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:128.0) Gecko/20100101 Firefox/128.0";

const BROWSERS: &[BrowserDef] = &[
    BrowserDef { id: "chrome", label: "Google Chrome", kind: BrowserKind::Chromium, base_env: "LOCALAPPDATA", rel_path: r"Google\Chrome\User Data", user_agent: CHROME_UA },
    BrowserDef { id: "edge", label: "Microsoft Edge", kind: BrowserKind::Chromium, base_env: "LOCALAPPDATA", rel_path: r"Microsoft\Edge\User Data", user_agent: EDGE_UA },
    BrowserDef { id: "brave", label: "Brave", kind: BrowserKind::Chromium, base_env: "LOCALAPPDATA", rel_path: r"BraveSoftware\Brave-Browser\User Data", user_agent: CHROME_UA },
    BrowserDef { id: "firefox", label: "Firefox", kind: BrowserKind::Firefox, base_env: "APPDATA", rel_path: r"Mozilla\Firefox", user_agent: FIREFOX_UA },
];

/// A browser the user has installed, for the frontend account picker.
#[derive(Debug, Serialize)]
pub struct BrowserInfo {
    pub id: String,
    pub label: String,
    /// Firefox Multi-Account Containers (empty for other browsers). One is
    /// picked by importing with the id `firefox:<container id>`.
    pub containers: Vec<FirefoxContainer>,
}

/// A Firefox Multi-Account Container.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct FirefoxContainer {
    /// `userContextId`; the default (container-less) tab is not one of these.
    pub id: u32,
    pub name: String,
}

/// Which Firefox cookie jar to read: the default one, or a container's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FirefoxJar {
    Default,
    Container(u32),
}

impl FirefoxJar {
    /// The exact `originAttributes` value of first-party cookies in this jar.
    /// (Partitioned third-party cookies carry extra attributes and are
    /// deliberately left out.)
    fn origin_attributes(self) -> String {
        match self {
            Self::Default => String::new(),
            Self::Container(id) => format!("^userContextId={id}"),
        }
    }
}

/// What a login on a given site looks like in a cookie jar.
pub struct SiteCookies {
    /// Human-readable site name for error messages.
    pub label: &'static str,
    /// The login cookie a session can't work without.
    pub primary: &'static str,
    /// Cookies a fully usable session also has (CSRF tokens etc.) — used to
    /// prefer one browser profile over another, not to limit what's read.
    pub extras: &'static [&'static str],
    /// SQL `LIKE` patterns matched against the cookie's host.
    pub hosts: &'static [&'static str],
}

impl SiteCookies {
    fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        std::iter::once(self.primary).chain(self.extras.iter().copied())
    }
}

pub const X: SiteCookies = SiteCookies {
    label: "X/Twitter",
    primary: "auth_token",
    extras: &["ct0"],
    hosts: &["%x.com", "%twitter.com"],
};

pub const INSTAGRAM: SiteCookies = SiteCookies {
    label: "Instagram",
    primary: "sessionid",
    extras: &["csrftoken", "ds_user_id"],
    hosts: &["%instagram.com"],
};

/// Credentials lifted from a browser, ready to hand to a scraper.
pub struct ImportedSession {
    /// Every cookie the browser holds for the site, primary first.
    cookies: Vec<(String, String)>,
    /// User-Agent of the browser the cookies came from.
    pub user_agent: &'static str,
}

impl ImportedSession {
    /// Format as the `name=value; name=value` cookie string the scrapers parse.
    pub fn as_cookie_string(&self) -> String {
        self.cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// How many of the spec's cookies (primary + extras) this session has.
    fn spec_hits(&self, site: &SiteCookies) -> usize {
        site.names()
            .filter(|name| self.cookies.iter().any(|(k, _)| k == name))
            .count()
    }

    /// Every cookie the site spec names was found.
    fn is_complete(&self, site: &SiteCookies) -> bool {
        self.spec_hits(site) == 1 + site.extras.len()
    }
}

/// Turn one profile's `name → value` cookies into a session, or `None` when
/// the primary login cookie is missing. Everything is kept: the spec's cookies
/// first, in spec order, then the rest by name.
fn assemble(
    site: &SiteCookies,
    d: &BrowserDef,
    mut found: HashMap<String, String>,
) -> Option<ImportedSession> {
    found.get(site.primary)?;
    let mut cookies: Vec<(String, String)> = site
        .names()
        .filter_map(|name| found.remove(name).map(|val| (name.to_string(), val)))
        .collect();
    let mut rest: Vec<(String, String)> = found.into_iter().collect();
    rest.sort();
    cookies.extend(rest);
    Some(ImportedSession { cookies, user_agent: d.user_agent })
}

fn def(id: &str) -> Option<&'static BrowserDef> {
    BROWSERS.iter().find(|b| b.id == id)
}

fn data_root(d: &BrowserDef) -> Option<PathBuf> {
    let base = std::env::var(d.base_env).ok()?;
    let p = Path::new(&base).join(d.rel_path);
    p.is_dir().then_some(p)
}

/// Browsers that are actually installed (have a data folder present), with
/// Firefox's containers attached.
pub fn list_installed() -> Vec<BrowserInfo> {
    BROWSERS
        .iter()
        .filter_map(|d| {
            let root = data_root(d)?;
            let containers = match d.kind {
                BrowserKind::Firefox => firefox_containers(&root),
                BrowserKind::Chromium => Vec::new(),
            };
            Some(BrowserInfo { id: d.id.to_string(), label: d.label.to_string(), containers })
        })
        .collect()
}

/// Import a site's auth cookies from the given browser id (`chrome`,
/// `firefox`, `firefox:<contextId>` …).
pub fn import(browser_id: &str, site: &SiteCookies) -> Result<ImportedSession, CookieError> {
    let unknown = || CookieError::Unknown(browser_id.to_string());
    let (base, container) = match browser_id.split_once(':') {
        Some((base, ctx)) => (base, Some(ctx.parse::<u32>().map_err(|_| unknown())?)),
        None => (browser_id, None),
    };
    let d = def(base).ok_or_else(unknown)?;
    // Only Firefox has containers.
    if d.kind != BrowserKind::Firefox && container.is_some() {
        return Err(unknown());
    }
    let root = data_root(d).ok_or_else(|| CookieError::NotInstalled(d.label.to_string()))?;
    match d.kind {
        BrowserKind::Chromium => import_chromium(d, &root, site),
        BrowserKind::Firefox => {
            let jar = container.map_or(FirefoxJar::Default, FirefoxJar::Container);
            import_firefox(d, &root, site, jar)
        }
    }
}

// --- Chromium ---------------------------------------------------------------

fn import_chromium(
    d: &BrowserDef,
    root: &Path,
    site: &SiteCookies,
) -> Result<ImportedSession, CookieError> {
    let key = chromium_master_key(d, root)?;

    let mut fallback: Option<ImportedSession> = None;
    let mut had_decrypt_error = false;

    for profile in chromium_profiles(root) {
        let db = profile.join("Network").join("Cookies");
        let db = if db.is_file() { db } else { profile.join("Cookies") };
        if !db.is_file() {
            continue;
        }
        let rows = read_db(d, &db, &CHROMIUM_SCHEMA, site, None)?;

        // Keep the cookies paired per profile — mixing them across profiles
        // would marry one account's login to another account's CSRF value.
        let mut found = HashMap::new();
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
            found.insert(name, val);
        }

        match assemble(site, d, found) {
            // A complete session — prefer it and stop.
            Some(s) if s.is_complete(site) => return Ok(s),
            // Login cookie but some extras missing: remember the fullest one as
            // a fallback, but keep looking for a profile that has them all.
            Some(s) if fallback.as_ref().is_none_or(|f| s.spec_hits(site) > f.spec_hits(site)) => {
                fallback = Some(s)
            }
            _ => {}
        }
    }

    fallback.ok_or_else(|| {
        if had_decrypt_error {
            CookieError::Decrypt(d.label.to_string())
        } else {
            CookieError::NoCookies { browser: d.label.to_string(), site: site.label }
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

fn import_firefox(
    d: &BrowserDef,
    root: &Path,
    site: &SiteCookies,
    jar: FirefoxJar,
) -> Result<ImportedSession, CookieError> {
    let origin_attributes = jar.origin_attributes();

    for profile in firefox_profiles(root) {
        let db = profile.join("cookies.sqlite");
        if !db.is_file() {
            continue;
        }
        let rows = read_db(d, &db, &FIREFOX_SCHEMA, site, Some(&origin_attributes))?;
        let found: HashMap<String, String> = rows
            .into_iter()
            .map(|(name, blob)| (name, String::from_utf8_lossy(&blob).into_owned()))
            .filter(|(_, val)| !val.is_empty())
            .collect();
        // First profile with a login wins.
        if let Some(s) = assemble(site, d, found) {
            return Ok(s);
        }
    }

    // Name the container in the error so "no login" points at the right jar.
    let browser = match jar {
        FirefoxJar::Default => d.label.to_string(),
        FirefoxJar::Container(id) => {
            let name = firefox_containers(root)
                .into_iter()
                .find(|c| c.id == id)
                .map(|c| c.name)
                .unwrap_or_else(|| format!("container {id}"));
            format!("{} · {name}", d.label)
        }
    };
    Err(CookieError::NoCookies { browser, site: site.label })
}

fn firefox_profiles(root: &Path) -> Vec<PathBuf> {
    fs::read_dir(root.join("Profiles"))
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

/// Multi-Account Containers defined across the profiles' `containers.json`,
/// by id. Built-in ones have an `l10nId` instead of a name.
fn firefox_containers(root: &Path) -> Vec<FirefoxContainer> {
    let mut by_id: BTreeMap<u32, String> = BTreeMap::new();
    for profile in firefox_profiles(root) {
        let Ok(text) = fs::read_to_string(profile.join("containers.json")) else { continue };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        for c in parse_containers(&json) {
            by_id.entry(c.id).or_insert(c.name);
        }
    }
    by_id
        .into_iter()
        .map(|(id, name)| FirefoxContainer { id, name })
        .collect()
}

fn parse_containers(json: &serde_json::Value) -> Vec<FirefoxContainer> {
    use serde_json::Value;

    json.get("identities")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        // `public: false` marks Firefox's internal pseudo-containers.
        .filter(|c| c.get("public").and_then(Value::as_bool) == Some(true))
        .filter_map(|c| {
            let id = c.get("userContextId").and_then(Value::as_u64)?;
            let id = u32::try_from(id).ok()?;
            let name = c
                .get("name")
                .and_then(Value::as_str)
                .filter(|n| !n.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    c.get("l10nId")
                        .or_else(|| c.get("l10nID"))
                        .and_then(Value::as_str)
                        .map(builtin_container_name)
                })
                .unwrap_or_else(|| format!("Container {id}"));
            Some(FirefoxContainer { id, name })
        })
        .collect()
}

/// English label for a built-in container's l10n id (`user-context-personal`
/// / `userContextPersonal.label` …).
fn builtin_container_name(l10n: &str) -> String {
    let key = l10n.to_ascii_lowercase();
    ["Personal", "Work", "Banking", "Shopping"]
        .into_iter()
        .find(|name| key.contains(&name.to_ascii_lowercase()))
        .map(str::to_string)
        .unwrap_or_else(|| l10n.to_string())
}

// --- SQLite helper ----------------------------------------------------------

/// Where a browser family keeps its cookies; the table and column names differ.
struct CookieSchema {
    table: &'static str,
    value_col: &'static str,
    host_col: &'static str,
    /// Column separating cookie jars (Firefox containers), if the browser has one.
    jar_col: Option<&'static str>,
}

const CHROMIUM_SCHEMA: CookieSchema = CookieSchema {
    table: "cookies",
    value_col: "encrypted_value",
    host_col: "host_key",
    jar_col: None,
};

const FIREFOX_SCHEMA: CookieSchema = CookieSchema {
    table: "moz_cookies",
    value_col: "value",
    host_col: "host",
    jar_col: Some("originAttributes"),
};

/// Copy the (possibly browser-locked) DB to a temp file and select every
/// cookie on the site's hosts — within one jar, when the schema has jars and
/// `jar` names it — returning `(name, value-bytes)` rows. Firefox `value` comes
/// back as text bytes; Chromium `encrypted_value` as the raw blob.
fn read_db(
    d: &BrowserDef,
    db: &Path,
    schema: &CookieSchema,
    site: &SiteCookies,
    jar: Option<&str>,
) -> Result<Vec<(String, Vec<u8>)>, CookieError> {
    use rusqlite::{params_from_iter, Connection, OpenFlags};

    // Hosts and the jar are bound as parameters; only the static schema
    // identifiers are interpolated.
    let host_marks = site
        .hosts
        .iter()
        .map(|_| format!("{} LIKE ?", schema.host_col))
        .collect::<Vec<_>>()
        .join(" OR ");
    let mut query = format!(
        "SELECT name, {} FROM {} WHERE ({host_marks})",
        schema.value_col, schema.table
    );
    let mut params: Vec<&str> = site.hosts.to_vec();
    if let (Some(col), Some(jar)) = (schema.jar_col, jar) {
        query.push_str(&format!(" AND {col} = ?"));
        params.push(jar);
    }

    let label = d.label.to_string();
    let copy = TempCopy::new(db).map_err(|e| CookieError::Db(label.clone(), e))?;

    let conn = Connection::open_with_flags(
        &copy.db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| CookieError::Db(label.clone(), e.to_string()))?;

    let mut stmt = conn
        .prepare(&query)
        .map_err(|e| CookieError::Db(label.clone(), e.to_string()))?;
    let rows = stmt
        .query_map(params_from_iter(params), |row| {
            let name: String = row.get(0)?;
            // Firefox stores `value` as TEXT while Chromium's `encrypted_value`
            // is a BLOB; `as_bytes` takes either.
            let value = row.get_ref(1)?.as_bytes()?.to_vec();
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
#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str, schema_sql: &str, rows: &[(&str, rusqlite::types::Value, &str, &str)]) -> PathBuf {
        let dir = unique_private_dir().unwrap();
        let path = dir.join(name);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(schema_sql).unwrap();
        for (n, v, h, jar) in rows {
            conn.execute("INSERT INTO t VALUES (?1, ?2, ?3, ?4)", rusqlite::params![n, v, h, jar]).unwrap();
        }
        path
    }

    fn firefox_def() -> &'static BrowserDef {
        def("firefox").unwrap()
    }

    #[test]
    fn reads_firefox_text_values_and_chromium_blobs() {
        use rusqlite::types::Value;

        // Firefox: `value` is TEXT. Every cookie on the site's hosts comes
        // back — but only from the requested jar (container).
        let ff = temp_db(
            "cookies.sqlite",
            "CREATE TABLE t (name TEXT, value TEXT, host TEXT, originAttributes TEXT); CREATE VIEW moz_cookies AS SELECT * FROM t;",
            &[
                ("sessionid", Value::Text("main".into()), ".instagram.com", ""),
                ("csrftoken", Value::Text("csrf".into()), "www.instagram.com", ""),
                ("sessionid", Value::Text("other-site".into()), ".example.com", ""),
                ("unrelated", Value::Text("x".into()), ".instagram.com", ""),
                ("sessionid", Value::Text("scraper".into()), ".instagram.com", "^userContextId=8"),
                ("sessionid", Value::Text("partitioned".into()), ".instagram.com", "^partitionKey=(https,example.com)&userContextId=8"),
            ],
        );
        let jar = FirefoxJar::Default.origin_attributes();
        let mut rows = read_db(firefox_def(), &ff, &FIREFOX_SCHEMA, &INSTAGRAM, Some(&jar)).unwrap();
        rows.sort();
        assert_eq!(
            rows,
            vec![
                ("csrftoken".to_string(), b"csrf".to_vec()),
                ("sessionid".to_string(), b"main".to_vec()),
                ("unrelated".to_string(), b"x".to_vec()),
            ]
        );
        let jar = FirefoxJar::Container(8).origin_attributes();
        let rows = read_db(firefox_def(), &ff, &FIREFOX_SCHEMA, &INSTAGRAM, Some(&jar)).unwrap();
        assert_eq!(rows, vec![("sessionid".to_string(), b"scraper".to_vec())]);

        // Chromium: `encrypted_value` is a BLOB, and no jar column.
        let ch = temp_db(
            "Cookies",
            "CREATE TABLE t (name TEXT, encrypted_value BLOB, host_key TEXT, unused TEXT); CREATE VIEW cookies AS SELECT * FROM t;",
            &[("auth_token", Value::Blob(vec![1, 2, 3]), ".x.com", "")],
        );
        let rows = read_db(firefox_def(), &ch, &CHROMIUM_SCHEMA, &X, None).unwrap();
        assert_eq!(rows, vec![("auth_token".to_string(), vec![1, 2, 3])]);

        for db in [ff, ch] {
            let _ = fs::remove_dir_all(db.parent().unwrap());
        }
    }

    #[test]
    fn lists_public_containers_with_builtin_names_translated() {
        let json = serde_json::json!({
            "version": 5,
            "identities": [
                { "userContextId": 1, "public": true, "l10nId": "user-context-personal" },
                { "userContextId": 3, "public": true, "l10nID": "userContextBanking.label", "name": "" },
                { "userContextId": 4, "public": true, "name": "ショッピング" },
                { "userContextId": 5, "public": false, "name": "userContextIdInternal.thumbnail" },
                { "userContextId": 8, "public": true, "name": "Twitter Sub Account" },
            ]
        });
        let names: Vec<(u32, String)> = parse_containers(&json).into_iter().map(|c| (c.id, c.name)).collect();
        assert_eq!(
            names,
            vec![
                (1, "Personal".to_string()),
                (3, "Banking".to_string()),
                (4, "ショッピング".to_string()),
                (8, "Twitter Sub Account".to_string()),
            ]
        );
    }

    #[test]
    fn container_ids_route_to_firefox_jars() {
        assert!(matches!(import("firefox:abc", &INSTAGRAM), Err(CookieError::Unknown(_))));
        assert!(matches!(import("chrome:1", &INSTAGRAM), Err(CookieError::Unknown(_))));
        assert_eq!(FirefoxJar::Container(8).origin_attributes(), "^userContextId=8");
    }

    #[test]
    fn assembles_a_session_only_around_the_primary_cookie() {
        let found: HashMap<String, String> = [("mid", "m"), ("csrftoken", "c"), ("sessionid", "s"), ("datr", "d")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let session = assemble(&INSTAGRAM, firefox_def(), found).unwrap();
        assert_eq!(session.as_cookie_string(), "sessionid=s; csrftoken=c; datr=d; mid=m");
        assert_eq!(session.spec_hits(&INSTAGRAM), 2);
        assert!(!session.is_complete(&INSTAGRAM), "ds_user_id is missing");
        assert!(session.user_agent.contains("Firefox"));

        let no_primary: HashMap<String, String> =
            [("csrftoken".to_string(), "c".to_string())].into_iter().collect();
        assert!(assemble(&INSTAGRAM, firefox_def(), no_primary).is_none());
    }
}
