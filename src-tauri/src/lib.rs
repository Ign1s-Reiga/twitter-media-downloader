mod cookies;
mod instagram;
mod media;
mod twitter;

use cookies::BrowserInfo;
use instagram::InstagramScraper;
use media::{MediaItem, MediaPage};
use serde::Serialize;
use twitter::TwitterScraper;

/// How many media items per page request to the platform.
const PAGE_SIZE: u32 = 20;
/// Default ceiling on items returned in a single `fetch_user_media` call, so a
/// prolific account doesn't spin forever.
const DEFAULT_MAX_ITEMS: usize = 100;
/// Safety cap on page requests regardless of `max_items`.
const MAX_PAGES: usize = 20;

/// Browser-like UA used when fetching media from a platform's CDN, which may
/// reject requests (notably video.twimg.com) that don't look like a browser.
const MEDIA_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Media CDNs we'll fetch from, and the Referer each one expects. Doubles as
/// the allowlist for `proxy_media` / `download_media` so neither can be turned
/// into an open proxy / SSRF. Hosts match exactly or as a subdomain, since
/// Instagram's CDN spreads across `scontent-*.cdninstagram.com` and
/// `*.fbcdn.net`.
const MEDIA_CDNS: [(&str, &str); 3] = [
    ("twimg.com", "https://x.com/"),
    ("cdninstagram.com", "https://www.instagram.com/"),
    ("fbcdn.net", "https://www.instagram.com/"),
];

/// The Referer to send for a media URL, or `None` if its host isn't an
/// allowlisted CDN (or the URL isn't https).
fn cdn_referer(url: &str) -> Option<&'static str> {
    let parsed = reqwest::Url::parse(url).ok()?;
    if parsed.scheme() != "https" {
        return None;
    }
    let host = parsed.host_str()?;
    MEDIA_CDNS
        .iter()
        .find(|(cdn, _)| host == *cdn || host.ends_with(&format!(".{cdn}")))
        .map(|(_, referer)| *referer)
}

/// Which site to scrape.
#[derive(Clone, Copy)]
enum Platform {
    X,
    Instagram,
}

impl Platform {
    /// `"x"` (also `"twitter"`) or `"instagram"`; absent means X, which is
    /// what the app did before it grew a second platform.
    fn parse(id: Option<&str>) -> Result<Self, String> {
        match id.map(str::trim).filter(|p| !p.is_empty()) {
            None | Some("x") | Some("twitter") => Ok(Self::X),
            Some("instagram") => Ok(Self::Instagram),
            Some(other) => Err(format!("unknown platform: {other}")),
        }
    }

    fn cookie_site(self) -> &'static cookies::SiteCookies {
        match self {
            Self::X => &cookies::X,
            Self::Instagram => &cookies::INSTAGRAM,
        }
    }
}

/// One scraper per platform, behind a common surface for the paging loop.
enum Scraper {
    X(TwitterScraper),
    Instagram(InstagramScraper),
}

impl Scraper {
    fn new(platform: Platform, session: Option<&str>) -> Result<Self, String> {
        Ok(match platform {
            Platform::X => Self::X(TwitterScraper::new(session).map_err(|e| e.to_string())?),
            Platform::Instagram => {
                Self::Instagram(InstagramScraper::new(session).map_err(|e| e.to_string())?)
            }
        })
    }

    /// Pull the account's username out of the pasted URL / handle.
    fn extract_username(platform: Platform, input: &str) -> Result<String, String> {
        match platform {
            Platform::X => twitter::extract_screen_name(input).map_err(|e| e.to_string()),
            Platform::Instagram => instagram::extract_username(input).map_err(|e| e.to_string()),
        }
    }

    async fn get_user_id(&mut self, username: &str) -> Result<String, String> {
        match self {
            Self::X(s) => s.get_user_id(username).await.map_err(|e| e.to_string()),
            Self::Instagram(s) => s.get_user_id(username).await.map_err(|e| e.to_string()),
        }
    }

    async fn fetch_media_page(
        &mut self,
        user_id: &str,
        cursor: Option<&str>,
        count: u32,
    ) -> Result<MediaPage, String> {
        match self {
            Self::X(s) => s
                .fetch_media_page(user_id, cursor, count)
                .await
                .map_err(|e| e.to_string()),
            Self::Instagram(s) => s
                .fetch_media_page(user_id, cursor, count)
                .await
                .map_err(|e| e.to_string()),
        }
    }
}

#[derive(Serialize)]
pub struct MediaResponse {
    items: Vec<MediaItem>,
    /// Cursor for a follow-up "load more" call, if the timeline has more.
    next_cursor: Option<String>,
}

/// List the browsers installed on this machine that we can import cookies from.
#[tauri::command]
fn list_browsers() -> Vec<BrowserInfo> {
    cookies::list_installed()
}

/// Scrape a user's media timeline.
///
/// * `platform` — `"x"` or `"instagram"` (defaults to X).
/// * `account_url` — profile URL, `@handle`, or bare handle.
/// * `filters` — any of `"images"` / `"videos"`; empty/None means no filtering.
/// * `browser` — a browser id (`chrome`/`edge`/`brave`/`firefox`) to import the
///   logged-in account's cookies from. Takes precedence over `session_token`.
/// * `session_token` — optional login cookie (`auth_token` for X, `sessionid`
///   for Instagram — or a full cookie string) for authenticated access; falls
///   back to guest mode when absent.
/// * `max_items` — soft cap on items to return (defaults to 100).
#[tauri::command]
async fn fetch_user_media(
    platform: Option<String>,
    account_url: String,
    filters: Option<Vec<String>>,
    browser: Option<String>,
    session_token: Option<String>,
    max_items: Option<usize>,
) -> Result<MediaResponse, String> {
    let platform = Platform::parse(platform.as_deref())?;
    let username = Scraper::extract_username(platform, &account_url)?;
    let cap = max_items.unwrap_or(DEFAULT_MAX_ITEMS).max(1);
    let wants = MediaFilter::from(filters.as_deref());

    // Browser import wins; otherwise a manually pasted token; otherwise guest.
    // Cookie import does blocking file/SQLite/DPAPI work — keep it off the async
    // runtime thread.
    let session = match browser.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        Some(id) => {
            let id = id.to_string();
            let site = platform.cookie_site();
            let imported = tauri::async_runtime::spawn_blocking(move || cookies::import(&id, site))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            Some(imported.as_cookie_string())
        }
        None => session_token.filter(|s| !s.trim().is_empty()),
    };

    let mut scraper = Scraper::new(platform, session.as_deref())?;
    let user_id = scraper.get_user_id(&username).await?;

    let mut items: Vec<MediaItem> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut next_cursor: Option<String> = None;

    for _ in 0..MAX_PAGES {
        let page = scraper
            .fetch_media_page(&user_id, cursor.as_deref(), PAGE_SIZE)
            .await?;

        let page_cursor = page.next_cursor.clone();
        for item in page.items {
            if wants.accepts(&item.media_type) {
                items.push(item);
            }
        }

        // `cap` is a soft floor: once reached, return the whole last page (don't
        // truncate — a future "load more" using next_cursor can't resume mid-page,
        // so truncating would drop those items) and hand back the cursor.
        if items.len() >= cap {
            next_cursor = page_cursor;
            break;
        }

        // Follow the cursor even on a page that yielded no media (X returns
        // cursor-only / interstitial pages; Instagram pages can be all
        // filtered out). Stop when there's no cursor or it stops advancing (X
        // echoes the same bottom cursor at the end).
        match page_cursor {
            Some(c) if Some(&c) != cursor.as_ref() => cursor = Some(c),
            _ => {
                next_cursor = None;
                break;
            }
        }
    }

    Ok(MediaResponse { items, next_cursor })
}

/// Which media kinds the caller asked for.
struct MediaFilter {
    images: bool,
    videos: bool,
}

impl MediaFilter {
    fn from(filters: Option<&[String]>) -> Self {
        match filters {
            // No selection means "everything".
            None => Self { images: true, videos: true },
            Some(f) if f.is_empty() => Self { images: true, videos: true },
            Some(f) => Self {
                images: f.iter().any(|x| x == "images"),
                videos: f.iter().any(|x| x == "videos"),
            },
        }
    }

    fn accepts(&self, media_type: &str) -> bool {
        match media_type {
            "photo" => self.images,
            "video" | "animated_gif" => self.videos,
            _ => true,
        }
    }
}

/// Download a single media URL into the OS Downloads folder. Returns the saved
/// absolute path so the UI can offer "show in folder".
#[tauri::command]
async fn download_media(
    app: tauri::AppHandle,
    url: String,
    filename: String,
    dir: Option<String>,
) -> Result<String, String> {
    use tauri::Manager;

    let referer = cdn_referer(&url).ok_or_else(|| "host not allowed".to_string())?;

    // Save into the user-specified folder when given (creating it if needed),
    // else the OS Downloads dir (which always exists).
    let base = match dir.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => {
            let p = std::path::PathBuf::from(d);
            std::fs::create_dir_all(&p).map_err(|e| e.to_string())?;
            p
        }
        None => app
            .path()
            .download_dir()
            .map_err(|e| format!("couldn't locate the Downloads folder: {e}"))?,
    };
    let dest = unique_path(&base, &sanitize_filename(&filename));

    let client = reqwest::Client::builder()
        .user_agent(MEDIA_UA)
        .build()
        .map_err(|e| e.to_string())?;
    let bytes = client
        .get(&url)
        .header("referer", referer)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;

    // Writing a potentially-large file is blocking; keep it off the async runtime.
    let write_dest = dest.clone();
    tauri::async_runtime::spawn_blocking(move || std::fs::write(&write_dest, &bytes))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    Ok(dest.to_string_lossy().into_owned())
}

/// Reduce a caller-supplied filename to a safe basename (no path separators).
fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('.').to_string();
    if cleaned.is_empty() {
        "media".to_string()
    } else {
        cleaned
    }
}

/// Avoid clobbering an existing file by inserting ` (n)` before the extension.
fn unique_path(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let path = std::path::Path::new(name);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_string());
    let ext = path
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();
    for n in 1..1000 {
        let candidate = dir.join(format!("{stem} ({n}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(name)
}

/// Stream platform media through a custom URI scheme so the webview plays it as
/// a same-origin resource. video.twimg.com returns 403 when the Referer is a
/// non-Twitter origin — and the webview's `<video>` element sends the app origin
/// (`http://tauri.localhost`) as Referer, which can't be overridden client-side.
/// So we fetch server-side with the CDN's own Referer, forward the Range header
/// for seeking, and relay the bytes.
async fn proxy_media(
    request: tauri::http::Request<Vec<u8>>,
) -> Result<tauri::http::Response<Vec<u8>>, String> {
    let encoded = request
        .uri()
        .query()
        .unwrap_or("")
        .split('&')
        .find_map(|kv| kv.strip_prefix("u="))
        .ok_or_else(|| "missing url".to_string())?;
    let target = urlencoding::decode(encoded)
        .map_err(|e| e.to_string())?
        .into_owned();

    // Only known media CDNs, so this can't act as an open proxy / SSRF.
    let referer = cdn_referer(&target).ok_or_else(|| "host not allowed".to_string())?;

    let client = reqwest::Client::new();
    let mut req = client
        .get(&target)
        .header("referer", referer)
        .header("user-agent", MEDIA_UA);
    if let Some(range) = request.headers().get("range").and_then(|v| v.to_str().ok()) {
        req = req.header("range", range);
    }
    let upstream = req.send().await.map_err(|e| e.to_string())?;

    let mut builder = tauri::http::Response::builder().status(upstream.status().as_u16());
    for name in [
        "content-type",
        "content-length",
        "content-range",
        "accept-ranges",
        "cache-control",
    ] {
        if let Some(v) = upstream.headers().get(name) {
            builder = builder.header(name, v.as_bytes());
        }
    }
    let body = upstream.bytes().await.map_err(|e| e.to_string())?.to_vec();
    builder.body(body).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .register_asynchronous_uri_scheme_protocol("twmedia", |_ctx, request, responder| {
            tauri::async_runtime::spawn(async move {
                let response = proxy_media(request).await.unwrap_or_else(|_| {
                    tauri::http::Response::builder()
                        .status(502)
                        .body(Vec::new())
                        .unwrap()
                });
                responder.respond(response);
            });
        })
        .invoke_handler(tauri::generate_handler![
            fetch_user_media,
            list_browsers,
            download_media
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
#[cfg(test)]
mod tests {
    use super::cdn_referer;

    #[test]
    fn cdn_referer_allows_known_media_hosts() {
        assert_eq!(cdn_referer("https://pbs.twimg.com/media/a.jpg?name=orig"), Some("https://x.com/"));
        assert_eq!(cdn_referer("https://video.twimg.com/ext_tw_video/1/pu/vid/a.mp4"), Some("https://x.com/"));
        assert_eq!(
            cdn_referer("https://scontent-nrt1-1.cdninstagram.com/v/t51/a.jpg?oh=1&oe=2"),
            Some("https://www.instagram.com/")
        );
        assert_eq!(
            cdn_referer("https://instagram.fnrt1-1.fna.fbcdn.net/v/a.mp4"),
            Some("https://www.instagram.com/")
        );
    }

    #[test]
    fn cdn_referer_rejects_everything_else() {
        for url in [
            "http://pbs.twimg.com/media/a.jpg",       // plain http
            "https://twimg.com.evil.example/a.jpg",   // suffix spoof
            "https://evil.example/?u=pbs.twimg.com",  // host in query only
            "https://notfbcdn.net/a.mp4",             // shares a suffix, not a subdomain
            "https://127.0.0.1/admin",
            "file:///C:/Windows/win.ini",
            "not a url",
        ] {
            assert_eq!(cdn_referer(url), None, "{url}");
        }
    }
}
