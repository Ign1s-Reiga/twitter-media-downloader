mod cookies;
mod twitter;

use cookies::BrowserInfo;
use serde::Serialize;
use twitter::{MediaItem, TwitterScraper};

/// How many media items per page request to X.
const PAGE_SIZE: u32 = 20;
/// Default ceiling on items returned in a single `fetch_user_media` call, so a
/// prolific account doesn't spin forever.
const DEFAULT_MAX_ITEMS: usize = 100;
/// Safety cap on page requests regardless of `max_items`.
const MAX_PAGES: usize = 20;

/// Browser-like UA used when fetching media from Twitter's CDN, which rejects
/// some requests (notably video.twimg.com) that don't look like a browser.
const MEDIA_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

#[derive(Serialize)]
pub struct MediaResponse {
    items: Vec<MediaItem>,
    /// Cursor for a follow-up "load more" call, if the timeline has more.
    next_cursor: Option<String>,
}

/// List the browsers installed on this machine that we can import X cookies from.
#[tauri::command]
fn list_browsers() -> Vec<BrowserInfo> {
    cookies::list_installed()
}

/// Scrape a user's media timeline.
///
/// * `account_url` — profile URL, `@handle`, or bare handle.
/// * `filters` — any of `"images"` / `"videos"`; empty/None means no filtering.
/// * `browser` — a browser id (`chrome`/`edge`/`brave`/`firefox`) to import the
///   logged-in account's cookies from. Takes precedence over `session_token`.
/// * `session_token` — optional `auth_token` (or full cookie string) for
///   authenticated access; falls back to guest mode when absent.
/// * `max_items` — soft cap on items to return (defaults to 100).
#[tauri::command]
async fn fetch_user_media(
    account_url: String,
    filters: Option<Vec<String>>,
    browser: Option<String>,
    session_token: Option<String>,
    max_items: Option<usize>,
) -> Result<MediaResponse, String> {
    let screen_name = twitter::extract_screen_name(&account_url).map_err(|e| e.to_string())?;
    let cap = max_items.unwrap_or(DEFAULT_MAX_ITEMS).max(1);
    let wants = MediaFilter::from(filters.as_deref());

    // Browser import wins; otherwise a manually pasted token; otherwise guest.
    // Cookie import does blocking file/SQLite/DPAPI work — keep it off the async
    // runtime thread.
    let session = match browser.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        Some(id) => {
            let id = id.to_string();
            let imported = tauri::async_runtime::spawn_blocking(move || cookies::import(&id))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            Some(imported.as_cookie_string())
        }
        None => session_token.filter(|s| !s.trim().is_empty()),
    };

    let mut scraper = TwitterScraper::new(session.as_deref()).map_err(|e| e.to_string())?;
    let user_id = scraper.get_user_id(&screen_name).await.map_err(|e| e.to_string())?;

    let mut items: Vec<MediaItem> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut next_cursor: Option<String> = None;

    for _ in 0..MAX_PAGES {
        let page = scraper
            .fetch_media_page(&user_id, cursor.as_deref(), PAGE_SIZE)
            .await
            .map_err(|e| e.to_string())?;

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
        // cursor-only / interstitial pages). Stop when there's no cursor or it
        // stops advancing (X echoes the same bottom cursor at the end).
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

    // Save into the user-specified folder when given, else the OS Downloads dir.
    let base = match dir.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => std::path::PathBuf::from(d),
        None => app
            .path()
            .download_dir()
            .map_err(|e| format!("couldn't locate the Downloads folder: {e}"))?,
    };
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let dest = unique_path(&base, &sanitize_filename(&filename));

    let client = reqwest::Client::builder()
        .user_agent(MEDIA_UA)
        .build()
        .map_err(|e| e.to_string())?;
    let bytes = client
        .get(&url)
        .header("referer", "https://x.com/")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;

    std::fs::write(&dest, &bytes).map_err(|e| e.to_string())?;
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

/// Stream Twitter media through a custom URI scheme so the webview plays it as a
/// same-origin resource. video.twimg.com returns 403 when the Referer is a
/// non-Twitter origin — and the webview's `<video>` element sends the app origin
/// (`http://tauri.localhost`) as Referer, which can't be overridden client-side.
/// So we fetch server-side with a Twitter Referer, forward the Range header for
/// seeking, and relay the bytes.
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

    // Allowlist Twitter media hosts so this can't act as an open proxy / SSRF.
    const ALLOWED: [&str; 3] = [
        "https://video.twimg.com/",
        "https://video-ft.twimg.com/",
        "https://pbs.twimg.com/",
    ];
    if !ALLOWED.iter().any(|p| target.starts_with(p)) {
        return Err("host not allowed".to_string());
    }

    let client = reqwest::Client::new();
    let mut req = client
        .get(&target)
        .header("referer", "https://x.com/")
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