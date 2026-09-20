//! Direct scraper for Instagram user media, talking to the private `/api/v1`
//! REST endpoints the web client uses. No external binary, no official API.
//!
//! Instagram requires a logged-in session for practically everything: the
//! caller supplies their `sessionid` cookie (browser import or the UI "Session
//! Token" field). Guest requests are attempted but Instagram nearly always
//! answers them with a login wall, which we surface as `LoginRequired`.
//!
//! Flow: `users/web_profile_info` resolves a username to its numeric id (and
//! tells us if the account is private), then `feed/user/{id}` pages through
//! the timeline with `max_id` cursors. Carousel posts are flattened into one
//! `MediaItem` per slide.
//!
//! NOTE: `APP_ID` is the web client's public `X-IG-App-ID`. It has been stable
//! for years but, like everything private, may rotate — grab a fresh one from
//! any `/api/v1/` request in DevTools → Network if requests start failing.

use serde_json::Value;

use crate::media::{MediaItem, MediaPage};

/// Public app id shipped in the Instagram web bundle (not a secret).
const APP_ID: &str = "936619743392459";

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

const API_BASE: &str = "https://www.instagram.com/api/v1";

/// Media types in Instagram's v1 schema.
const MEDIA_TYPE_IMAGE: u64 = 1;
const MEDIA_TYPE_VIDEO: u64 = 2;
const MEDIA_TYPE_CAROUSEL: u64 = 8;

/// Preferred minimum width for grid thumbnails (see `smallest_image_at_least`).
const THUMB_MIN_WIDTH: u64 = 320;

#[derive(Debug, thiserror::Error)]
pub enum ScrapeError {
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("could not read a username from that input")]
    InvalidInput,
    #[error("user not found")]
    UserNotFound,
    #[error("this account is private — follow it from the account you're scraping with")]
    Private,
    #[error("Instagram requires a login — pick a browser you're logged into, or paste a sessionid cookie")]
    LoginRequired,
    #[error("Instagram is rate-limiting this account — wait a few minutes and try again")]
    RateLimited,
    #[error("Instagram returned an error: {0}")]
    Api(String),
    #[error("could not parse Instagram's response (the API schema may have changed)")]
    Parse,
}

/// Pull a bare username out of whatever the user pasted: a full profile URL,
/// an `@handle`, or just the handle.
pub fn extract_username(input: &str) -> Result<String, ScrapeError> {
    let raw = input.trim();
    if raw.is_empty() {
        return Err(ScrapeError::InvalidInput);
    }

    let candidate = if raw.contains("instagram.com") {
        // Strip scheme/host, then take the first path segment.
        let after_host = raw.split("instagram.com/").nth(1).unwrap_or(raw);
        after_host
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("")
            .to_string()
    } else {
        raw.trim_start_matches('@').to_string()
    };

    let name = candidate.trim_start_matches('@').trim();
    // Instagram usernames are 1-30 chars of [A-Za-z0-9._]. Reject reserved path
    // words so a pasted post/reel link doesn't get treated as a profile.
    let valid = !name.is_empty()
        && name.len() <= 30
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.'))
        && !matches!(
            name,
            "p" | "reel" | "reels" | "stories" | "explore" | "accounts" | "direct" | "tv"
        );
    if valid {
        Ok(name.to_string())
    } else {
        Err(ScrapeError::InvalidInput)
    }
}

/// Parsed credentials for an authenticated session.
struct Session {
    sessionid: String,
    /// CSRF cookie; sent back as `x-csrftoken` when we have it. GET endpoints
    /// don't enforce it, so it's optional.
    csrftoken: Option<String>,
    ds_user_id: Option<String>,
}

impl Session {
    fn cookie_header(&self) -> String {
        let mut parts = vec![format!("sessionid={}", self.sessionid)];
        if let Some(c) = &self.csrftoken {
            parts.push(format!("csrftoken={c}"));
        }
        if let Some(u) = &self.ds_user_id {
            parts.push(format!("ds_user_id={u}"));
        }
        parts.join("; ")
    }
}

/// Turn the UI "Session Token" field into a `Session`. Accepts either a bare
/// `sessionid` value or a full `key=value; key=value` cookie string.
fn parse_session(token: &str) -> Option<Session> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }

    // A bare sessionid looks like `12345678%3Aabcdef...` — it contains `%3A`
    // (or a literal `:`), never `=`. A cookie string always has `=`.
    if !token.contains('=') {
        return Some(Session {
            sessionid: token.to_string(),
            csrftoken: None,
            ds_user_id: None,
        });
    }

    let mut sessionid = None;
    let mut csrftoken = None;
    let mut ds_user_id = None;
    for pair in token.split(';') {
        let mut kv = pair.splitn(2, '=');
        let key = kv.next().unwrap_or("").trim();
        let val = kv.next().unwrap_or("").trim();
        if val.is_empty() {
            continue;
        }
        match key {
            "sessionid" => sessionid = Some(val.to_string()),
            "csrftoken" => csrftoken = Some(val.to_string()),
            "ds_user_id" => ds_user_id = Some(val.to_string()),
            _ => {}
        }
    }
    Some(Session {
        sessionid: sessionid?,
        csrftoken,
        ds_user_id,
    })
}

pub struct InstagramScraper {
    http: reqwest::Client,
    session: Option<Session>,
}

impl InstagramScraper {
    pub fn new(session_token: Option<&str>) -> Result<Self, ScrapeError> {
        // Don't follow redirects: an unauthenticated request gets bounced to
        // the login page with a 200 HTML body, which would be indistinguishable
        // from a real answer. A 3xx is a clean "login required" signal instead.
        let http = reqwest::Client::builder()
            .user_agent(UA)
            .gzip(true)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            http,
            session: session_token.and_then(parse_session),
        })
    }

    /// Apply the headers the web client sends on every `/api/v1` request.
    fn headers(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let req = req
            .header("accept", "*/*")
            .header("x-ig-app-id", APP_ID)
            .header("x-requested-with", "XMLHttpRequest")
            .header("origin", "https://www.instagram.com")
            .header("referer", "https://www.instagram.com/");
        match &self.session {
            Some(s) => {
                let req = req.header("cookie", s.cookie_header());
                match &s.csrftoken {
                    Some(c) => req.header("x-csrftoken", c),
                    None => req,
                }
            }
            None => req,
        }
    }

    async fn get_json(&self, url: &str) -> Result<Value, ScrapeError> {
        let resp = self.headers(self.http.get(url)).send().await?;
        let status = resp.status();
        if status.is_redirection() {
            return Err(ScrapeError::LoginRequired);
        }
        let text = resp.text().await?;
        let body: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            // A non-JSON body on an error status is the HTML login/challenge
            // wall; on a success status it's a schema change.
            Err(_) if status.is_success() => return Err(ScrapeError::Parse),
            Err(_) => return Err(self.classify_error(status, "")),
        };

        if status.is_success() {
            return Ok(body);
        }
        let msg = body
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        Err(self.classify_error(status, &msg))
    }

    /// Map a failed response to the most specific error we can name.
    fn classify_error(&self, status: reqwest::StatusCode, message: &str) -> ScrapeError {
        let lower = message.to_ascii_lowercase();
        let code = status.as_u16();
        let rate_limited = code == 429 || lower.contains("please wait a few minutes");
        let login_wall = matches!(code, 401 | 403)
            || lower.contains("login_required")
            || lower.contains("checkpoint_required")
            || lower.contains("challenge_required");

        // Without a session Instagram answers with 401s and, for profile
        // lookups, a "rate limit" 429 that's really a login wall — either way
        // the fix is to log in, so say so instead of "wait a few minutes".
        if login_wall || (rate_limited && self.session.is_none()) {
            return ScrapeError::LoginRequired;
        }
        if rate_limited {
            return ScrapeError::RateLimited;
        }
        if message.is_empty() {
            ScrapeError::Api(format!("HTTP {code}"))
        } else {
            ScrapeError::Api(format!("HTTP {code}: {message}"))
        }
    }

    /// Resolve a username to its numeric user id. Errors early on a private
    /// account we don't follow, since the feed endpoint would just come back
    /// empty for it.
    pub async fn get_user_id(&self, username: &str) -> Result<String, ScrapeError> {
        let url = format!(
            "{API_BASE}/users/web_profile_info/?username={}",
            urlencoding::encode(username)
        );
        let body = match self.get_json(&url).await {
            // Instagram 404s an unknown username rather than returning null.
            Err(ScrapeError::Api(m)) if m.starts_with("HTTP 404") => {
                return Err(ScrapeError::UserNotFound)
            }
            other => other?,
        };

        let user = match body.pointer("/data/user") {
            Some(Value::Null) | None => return Err(ScrapeError::UserNotFound),
            Some(u) => u,
        };
        let flag = |key: &str| user.get(key).and_then(Value::as_bool).unwrap_or(false);
        if flag("is_private") && !flag("followed_by_viewer") && !flag("viewer_is_self") {
            return Err(ScrapeError::Private);
        }

        user.get("id")
            .and_then(value_as_string)
            .ok_or(ScrapeError::Parse)
    }

    /// Fetch one page of a user's feed.
    pub async fn fetch_media_page(
        &self,
        user_id: &str,
        cursor: Option<&str>,
        count: u32,
    ) -> Result<MediaPage, ScrapeError> {
        let mut url = format!("{API_BASE}/feed/user/{user_id}/?count={count}");
        if let Some(c) = cursor {
            url.push_str("&max_id=");
            url.push_str(&urlencoding::encode(c));
        }
        let body = self.get_json(&url).await?;

        let items = body
            .get("items")
            .and_then(Value::as_array)
            .ok_or(ScrapeError::Parse)?;

        let mut out = Vec::new();
        for item in items {
            collect_post(item, &mut out);
        }

        let more = body
            .get("more_available")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let next_cursor = if more {
            body.get("next_max_id").and_then(value_as_string)
        } else {
            None
        };

        Ok(MediaPage { items: out, next_cursor })
    }
}

/// Process one feed post: a single image/video, or a carousel of them.
fn collect_post(post: &Value, out: &mut Vec<MediaItem>) {
    let code = post.get("code").and_then(Value::as_str).unwrap_or("");
    let post_url = format!("https://www.instagram.com/p/{code}/");

    match post.get("media_type").and_then(Value::as_u64) {
        Some(MEDIA_TYPE_CAROUSEL) => {
            let slides = post
                .get("carousel_media")
                .and_then(Value::as_array)
                .into_iter()
                .flatten();
            for slide in slides {
                if let Some(item) = build_media_item(slide, code, &post_url) {
                    out.push(item);
                }
            }
        }
        _ => {
            if let Some(item) = build_media_item(post, code, &post_url) {
                out.push(item);
            }
        }
    }
}

fn build_media_item(m: &Value, post_id: &str, post_url: &str) -> Option<MediaItem> {
    let id = m
        .get("pk")
        .and_then(value_as_string)
        .or_else(|| m.get("id").and_then(value_as_string))
        .unwrap_or_else(|| post_id.to_string());

    let is_video = match m.get("media_type").and_then(Value::as_u64) {
        Some(MEDIA_TYPE_VIDEO) => true,
        Some(MEDIA_TYPE_IMAGE) => false,
        // Unknown type: trust the presence of video renditions.
        _ => m.get("video_versions").is_some(),
    };

    // `image_versions2.candidates` holds the still image in several sizes (for
    // videos too — that's the poster frame).
    let candidates = m
        .pointer("/image_versions2/candidates")
        .and_then(Value::as_array)?;
    let full = largest_image(candidates)?;
    let thumb = smallest_image_at_least(candidates, THUMB_MIN_WIDTH).unwrap_or(full);

    let thumbnail_url = thumb.get("url").and_then(Value::as_str)?.to_string();
    let full_url = full.get("url").and_then(Value::as_str)?.to_string();

    let (width, height) = m
        .get("original_width")
        .and_then(Value::as_u64)
        .zip(m.get("original_height").and_then(Value::as_u64))
        .or_else(|| {
            full.get("width")
                .and_then(Value::as_u64)
                .zip(full.get("height").and_then(Value::as_u64))
        })
        .unwrap_or((0, 0));

    let (media_type, download_url) = if is_video {
        ("video", best_video_version(m).unwrap_or_else(|| full_url.clone()))
    } else {
        ("photo", full_url)
    };

    Some(MediaItem {
        id,
        post_id: post_id.to_string(),
        media_type: media_type.to_string(),
        thumbnail_url,
        download_url,
        post_url: post_url.to_string(),
        width,
        height,
    })
}

fn image_width(c: &Value) -> u64 {
    c.get("width").and_then(Value::as_u64).unwrap_or(0)
}

/// The full-resolution rendition. Candidates aren't guaranteed to be sorted.
fn largest_image(candidates: &[Value]) -> Option<&Value> {
    candidates.iter().max_by_key(|c| image_width(c))
}

/// The lightest rendition that's still at least `min_width` wide — a grid
/// thumbnail shouldn't pull the 1080px original.
fn smallest_image_at_least(candidates: &[Value], min_width: u64) -> Option<&Value> {
    candidates
        .iter()
        .filter(|c| image_width(c) >= min_width)
        .min_by_key(|c| image_width(c))
}

/// Pick the widest rendition from a video's `video_versions`.
fn best_video_version(m: &Value) -> Option<String> {
    let versions = m.get("video_versions")?.as_array()?;
    versions
        .iter()
        .max_by_key(|v| v.get("width").and_then(Value::as_u64).unwrap_or(0))
        .and_then(|v| v.get("url").and_then(Value::as_str))
        .map(str::to_string)
}

/// Instagram ids (`pk`, `id`, `next_max_id`) show up as either JSON strings or
/// numbers depending on the endpoint — normalise both to a string.
fn value_as_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_username_from_urls_and_handles() {
        for input in [
            "https://www.instagram.com/some.user_1/",
            "instagram.com/some.user_1/reels/?hl=en",
            "@some.user_1",
            "  some.user_1  ",
        ] {
            assert_eq!(extract_username(input).unwrap(), "some.user_1", "{input}");
        }
    }

    #[test]
    fn rejects_non_profile_input() {
        for input in [
            "",
            "https://www.instagram.com/p/C1a2B3c4D5e/",
            "https://www.instagram.com/reel/C1a2B3c4D5e/",
            "has space",
            "a".repeat(31).as_str(),
        ] {
            assert!(matches!(extract_username(input), Err(ScrapeError::InvalidInput)), "{input:?}");
        }
    }

    #[test]
    fn parses_bare_and_cookie_string_tokens() {
        let bare = parse_session("123%3Aabc").unwrap();
        assert_eq!(bare.sessionid, "123%3Aabc");
        assert!(bare.csrftoken.is_none());

        let full = parse_session("csrftoken=tok; sessionid=123%3Aabc; ds_user_id=123; other=x").unwrap();
        assert_eq!(full.sessionid, "123%3Aabc");
        assert_eq!(full.csrftoken.as_deref(), Some("tok"));
        assert_eq!(full.ds_user_id.as_deref(), Some("123"));
        assert_eq!(full.cookie_header(), "sessionid=123%3Aabc; csrftoken=tok; ds_user_id=123");

        assert!(parse_session("").is_none());
        assert!(parse_session("csrftoken=tok").is_none(), "no sessionid → no session");
    }

    fn candidates(widths: &[u64]) -> Value {
        let list: Vec<Value> = widths
            .iter()
            .map(|w| json!({ "width": w, "height": w, "url": format!("https://scontent.cdninstagram.com/{w}.jpg") }))
            .collect();
        json!({ "candidates": list })
    }

    #[test]
    fn parses_image_video_and_carousel_posts() {
        let mut out = Vec::new();

        // Single image; `pk` as a number.
        collect_post(
            &json!({
                "code": "IMG",
                "media_type": 1,
                "pk": 111,
                "original_width": 1080,
                "original_height": 1350,
                "image_versions2": candidates(&[1080, 640, 240]),
            }),
            &mut out,
        );
        // Video; `pk` as a string, widest rendition wins.
        collect_post(
            &json!({
                "code": "VID",
                "media_type": 2,
                "pk": "222",
                "image_versions2": candidates(&[720]),
                "video_versions": [
                    { "width": 480, "url": "https://scontent.cdninstagram.com/480.mp4" },
                    { "width": 720, "url": "https://scontent.cdninstagram.com/720.mp4" },
                ],
            }),
            &mut out,
        );
        // Carousel: one item per slide, all sharing the parent's post id/url.
        collect_post(
            &json!({
                "code": "CAR",
                "media_type": 8,
                "pk": 333,
                "carousel_media": [
                    { "pk": 3331, "media_type": 1, "image_versions2": candidates(&[1080]) },
                    { "pk": 3332, "media_type": 2, "image_versions2": candidates(&[1080]),
                      "video_versions": [{ "width": 1080, "url": "https://scontent.cdninstagram.com/c.mp4" }] },
                ],
            }),
            &mut out,
        );

        assert_eq!(out.len(), 4);

        let img = &out[0];
        assert_eq!((img.id.as_str(), img.post_id.as_str(), img.media_type.as_str()), ("111", "IMG", "photo"));
        assert_eq!(img.post_url, "https://www.instagram.com/p/IMG/");
        assert_eq!(img.download_url, "https://scontent.cdninstagram.com/1080.jpg");
        assert_eq!(img.thumbnail_url, "https://scontent.cdninstagram.com/640.jpg", "smallest ≥ 320px");
        assert_eq!((img.width, img.height), (1080, 1350));

        let vid = &out[1];
        assert_eq!((vid.id.as_str(), vid.media_type.as_str()), ("222", "video"));
        assert_eq!(vid.download_url, "https://scontent.cdninstagram.com/720.mp4");
        assert_eq!(vid.thumbnail_url, "https://scontent.cdninstagram.com/720.jpg", "poster frame");
        assert_eq!((vid.width, vid.height), (720, 720), "falls back to the candidate's size");

        assert_eq!(out[2].id, "3331");
        assert_eq!(out[3].id, "3332");
        assert_eq!(out[3].media_type, "video");
        assert!(out[2..].iter().all(|i| i.post_id == "CAR" && i.post_url.ends_with("/p/CAR/")));
    }

    #[test]
    fn skips_posts_without_images() {
        let mut out = Vec::new();
        collect_post(&json!({ "code": "X", "media_type": 1, "pk": 1 }), &mut out);
        assert!(out.is_empty());
    }
}
