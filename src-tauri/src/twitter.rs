//! Direct scraper for X (Twitter) user media, talking to X's internal GraphQL
//! API the same way the web client does. No external binary, no official API.
//!
//! Two auth modes:
//!   * Guest   — activates a guest token from the public web Bearer. Works for
//!               public accounts but X increasingly rate-limits / blocks guests.
//!   * Session — caller supplies their `auth_token` cookie (the UI "Session
//!               Token" field). We pair it with a `ct0` CSRF value using the
//!               double-submit-cookie scheme X expects.
//!
//! NOTE: `BEARER`, the GraphQL query IDs and the `features` blobs below are
//! lifted from the live web client and ROTATE over time. When scraping starts
//! failing with 404/400, refresh them first: open x.com in a browser, find the
//! `UserByScreenName` / `UserMedia` requests in DevTools → Network, and copy the
//! query-id path segment plus the `features` query param.

use rand::Rng;
use serde::Serialize;
use serde_json::{json, Value};

/// Public Bearer token shipped in the X web bundle (not a secret).
const BEARER: &str = "AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA";

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

const GRAPHQL_BASE: &str = "https://x.com/i/api/graphql";

// --- GraphQL query IDs (rotate; see module note) -----------------------------
const QID_USER_BY_SCREEN_NAME: &str = "681MIj51w00Aj6dY0GXnHw";
const QID_USER_MEDIA: &str = "Ecl7YvFIuRaUPonVOHzoOA";

#[derive(Debug, thiserror::Error)]
pub enum ScrapeError {
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("could not read a username from that input")]
    InvalidInput,
    #[error("user not found")]
    UserNotFound,
    #[error("this account is protected, suspended, or unavailable")]
    Unavailable,
    #[error("X returned an error: {0}")]
    Api(String),
    #[error("could not parse X's response (the GraphQL schema may have changed)")]
    Parse,
}

/// A single media attachment ready for the frontend to preview / download.
#[derive(Debug, Serialize)]
pub struct MediaItem {
    /// Stable media key from X (used as a React key / dedupe key).
    pub id: String,
    pub tweet_id: String,
    /// "photo" | "video" | "animated_gif".
    pub media_type: String,
    /// Smaller URL suitable for grid previews.
    pub thumbnail_url: String,
    /// Full-resolution image, or the best video/gif variant to download.
    pub download_url: String,
    pub tweet_url: String,
    pub width: u64,
    pub height: u64,
}

/// One page of results plus the cursor to fetch the next page.
#[derive(Debug, Serialize)]
pub struct MediaPage {
    pub items: Vec<MediaItem>,
    pub next_cursor: Option<String>,
}

/// Pull a bare screen name out of whatever the user pasted: a full profile URL,
/// an `@handle`, or just the handle.
pub fn extract_screen_name(input: &str) -> Result<String, ScrapeError> {
    let raw = input.trim();
    if raw.is_empty() {
        return Err(ScrapeError::InvalidInput);
    }

    let candidate = if raw.contains("twitter.com") || raw.contains("x.com") {
        // Strip scheme/host, then take the first path segment.
        let after_host = raw
            .split("twitter.com/")
            .nth(1)
            .or_else(|| raw.split("x.com/").nth(1))
            .unwrap_or(raw);
        after_host
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("")
            .to_string()
    } else {
        raw.trim_start_matches('@').to_string()
    };

    let name = candidate.trim_start_matches('@').trim();
    // X handles are 1-15 chars of [A-Za-z0-9_]. Reject reserved path words.
    let valid = !name.is_empty()
        && name.len() <= 15
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !matches!(name, "home" | "explore" | "search" | "i" | "messages");
    if valid {
        Ok(name.to_string())
    } else {
        Err(ScrapeError::InvalidInput)
    }
}

/// Parsed credentials for an authenticated session.
struct Session {
    auth_token: String,
    ct0: String,
}

/// Turn the UI "Session Token" field into a `Session`. Accepts either a bare
/// `auth_token` value or a full `key=value; key=value` cookie string. When no
/// `ct0` is supplied we mint a random one (X validates that the `ct0` cookie and
/// `x-csrf-token` header match, not that the server issued it).
fn parse_session(token: &str) -> Option<Session> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }

    if token.contains('=') {
        let mut auth_token = None;
        let mut ct0 = None;
        for pair in token.split(';') {
            let mut kv = pair.splitn(2, '=');
            let key = kv.next().unwrap_or("").trim();
            let val = kv.next().unwrap_or("").trim();
            match key {
                "auth_token" => auth_token = Some(val.to_string()),
                "ct0" => ct0 = Some(val.to_string()),
                _ => {}
            }
        }
        let auth_token = auth_token?;
        if auth_token.is_empty() {
            return None;
        }
        Some(Session {
            ct0: ct0.filter(|c| !c.is_empty()).unwrap_or_else(random_ct0),
            auth_token,
        })
    } else {
        Some(Session {
            auth_token: token.to_string(),
            ct0: random_ct0(),
        })
    }
}

fn random_ct0() -> String {
    let mut rng = rand::thread_rng();
    (0..32)
        .map(|_| format!("{:x}", rng.gen_range(0..16)))
        .collect()
}

pub struct TwitterScraper {
    http: reqwest::Client,
    session: Option<Session>,
    guest_token: Option<String>,
}

impl TwitterScraper {
    pub fn new(session_token: Option<&str>) -> Result<Self, ScrapeError> {
        let http = reqwest::Client::builder()
            .user_agent(UA)
            .gzip(true)
            .build()?;
        Ok(Self {
            http,
            session: session_token.and_then(parse_session),
            guest_token: None,
        })
    }

    /// Ensure we have *some* credential. In guest mode this activates a guest
    /// token; in session mode it's a no-op.
    async fn ensure_auth(&mut self) -> Result<(), ScrapeError> {
        if self.session.is_some() || self.guest_token.is_some() {
            return Ok(());
        }
        let resp = self
            .http
            .post("https://api.x.com/1.1/guest/activate.json")
            .header("authorization", format!("Bearer {BEARER}"))
            .send()
            .await?
            .error_for_status()?;
        let body: Value = resp.json().await?;
        let token = body
            .get("guest_token")
            .and_then(Value::as_str)
            .ok_or(ScrapeError::Parse)?;
        self.guest_token = Some(token.to_string());
        Ok(())
    }

    /// Apply the auth headers (Bearer + guest-token or session cookies) shared by
    /// every GraphQL request.
    fn auth_headers(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let req = req
            .header("authorization", format!("Bearer {BEARER}"))
            .header("x-twitter-active-user", "yes")
            .header("x-twitter-client-language", "en")
            .header("origin", "https://x.com")
            .header("referer", "https://x.com/");
        match &self.session {
            Some(s) => req
                .header("x-twitter-auth-type", "OAuth2Session")
                .header("x-csrf-token", &s.ct0)
                .header(
                    "cookie",
                    format!("auth_token={}; ct0={}", s.auth_token, s.ct0),
                ),
            None => {
                let guest = self.guest_token.as_deref().unwrap_or_default();
                req.header("x-guest-token", guest)
                    .header("cookie", format!("gt={guest}"))
            }
        }
    }

    async fn graphql_get(
        &self,
        query_id: &str,
        op: &str,
        variables: &Value,
        features: &Value,
    ) -> Result<Value, ScrapeError> {
        let url = format!(
            "{GRAPHQL_BASE}/{query_id}/{op}?variables={}&features={}",
            urlencoding::encode(&variables.to_string()),
            urlencoding::encode(&features.to_string()),
        );
        let resp = self.auth_headers(self.http.get(&url)).send().await?;
        let status = resp.status();
        let body: Value = resp.json().await?;
        if !status.is_success() {
            let msg = body
                .get("errors")
                .and_then(|e| e.get(0))
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
            return Err(ScrapeError::Api(msg));
        }
        Ok(body)
    }

    /// Resolve a screen name to its numeric `rest_id`.
    pub async fn get_user_id(&mut self, screen_name: &str) -> Result<String, ScrapeError> {
        self.ensure_auth().await?;
        let variables = json!({
            "screen_name": screen_name,
            "withSafetyModeUserFields": true,
        });
        let features = json!({
            "hidden_profile_subscriptions_enabled": true,
            "rweb_tipjar_consumption_enabled": true,
            "responsive_web_graphql_exclude_directive_enabled": true,
            "verified_phone_label_enabled": false,
            "subscriptions_verification_info_is_identity_verified_enabled": true,
            "subscriptions_verification_info_verified_since_enabled": true,
            "highlights_tweets_tab_ui_enabled": true,
            "responsive_web_twitter_article_notes_tab_enabled": true,
            "subscriptions_feature_can_gift_premium": true,
            "creator_subscriptions_tweet_preview_api_enabled": true,
            "responsive_web_graphql_skip_user_profile_image_extensions_enabled": false,
            "responsive_web_graphql_timeline_navigation_enabled": true,
        });
        let body = self
            .graphql_get(QID_USER_BY_SCREEN_NAME, "UserByScreenName", &variables, &features)
            .await?;

        let result = body
            .pointer("/data/user/result")
            .ok_or(ScrapeError::UserNotFound)?;
        match result.get("__typename").and_then(Value::as_str) {
            Some("UserUnavailable") => Err(ScrapeError::Unavailable),
            _ => result
                .get("rest_id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or(ScrapeError::UserNotFound),
        }
    }

    /// Fetch one page of a user's media timeline.
    pub async fn fetch_media_page(
        &mut self,
        user_id: &str,
        cursor: Option<&str>,
        count: u32,
    ) -> Result<MediaPage, ScrapeError> {
        self.ensure_auth().await?;
        let mut variables = json!({
            "userId": user_id,
            "count": count,
            "includePromotedContent": false,
            "withClientEventToken": false,
            "withBirdwatchNotes": false,
            "withVoice": true,
            "withV2Timeline": true,
        });
        if let Some(c) = cursor {
            variables["cursor"] = json!(c);
        }
        let features = media_features();
        let body = self
            .graphql_get(QID_USER_MEDIA, "UserMedia", &variables, &features)
            .await?;

        let instructions = match body
            .pointer("/data/user/result/timeline_v2/timeline/instructions")
            .or_else(|| body.pointer("/data/user/result/timeline/timeline/instructions"))
            .and_then(Value::as_array)
        {
            Some(i) => i,
            // A resolved user with no media timeline (no posts, or restricted to
            // us) is just zero items — not a schema/parse failure.
            None if body.pointer("/data/user/result").is_some() => {
                return Ok(MediaPage { items: Vec::new(), next_cursor: None })
            }
            None => return Err(ScrapeError::Parse),
        };

        let mut items = Vec::new();
        let mut next_cursor = None;
        for instruction in instructions {
            // Entries can live on a top-level add, or be appended to a module.
            let entries = instruction
                .get("entries")
                .and_then(Value::as_array)
                .into_iter()
                .flatten();
            let module_items = instruction
                .get("moduleItems")
                .and_then(Value::as_array)
                .into_iter()
                .flatten();

            for entry in entries {
                collect_entry(entry, &mut items, &mut next_cursor);
            }
            for item in module_items {
                collect_item_content(item.get("item"), &mut items);
            }
        }

        Ok(MediaPage { items, next_cursor })
    }
}

/// Process one timeline entry: a media-grid module, a single tweet, or a cursor.
fn collect_entry(entry: &Value, out: &mut Vec<MediaItem>, cursor: &mut Option<String>) {
    let content = match entry.get("content") {
        Some(c) => c,
        None => return,
    };
    match content.get("entryType").and_then(Value::as_str) {
        Some("TimelineTimelineCursor") => {
            if content.get("cursorType").and_then(Value::as_str) == Some("Bottom") {
                if let Some(v) = content.get("value").and_then(Value::as_str) {
                    *cursor = Some(v.to_string());
                }
            }
        }
        Some("TimelineTimelineModule") => {
            if let Some(grid) = content.get("items").and_then(Value::as_array) {
                for it in grid {
                    collect_item_content(it.get("item"), out);
                }
            }
        }
        Some("TimelineTimelineItem") => collect_item_content(content.get("itemContent"), out),
        _ => {
            // Some shapes nest itemContent directly on content.
            if content.get("itemContent").is_some() {
                collect_item_content(content.get("itemContent"), out);
            }
        }
    }
}

/// `item` here is `{ itemContent: { tweet_results: { result: <Tweet> } } }`.
fn collect_item_content(item: Option<&Value>, out: &mut Vec<MediaItem>) {
    let item_content = match item.and_then(|i| i.get("itemContent")).or(item) {
        Some(c) => c,
        None => return,
    };
    let result = match item_content.pointer("/tweet_results/result") {
        Some(r) => r,
        None => return,
    };
    // `TweetWithVisibilityResults` wraps the real tweet under `.tweet`.
    let tweet = result.get("tweet").unwrap_or(result);
    extract_media_from_tweet(tweet, out);
}

fn extract_media_from_tweet(tweet: &Value, out: &mut Vec<MediaItem>) {
    let legacy = match tweet.get("legacy") {
        Some(l) => l,
        None => return,
    };
    let tweet_id = legacy
        .get("id_str")
        .and_then(Value::as_str)
        .or_else(|| tweet.get("rest_id").and_then(Value::as_str))
        .unwrap_or("")
        .to_string();
    let screen_name = tweet
        .pointer("/core/user_results/result/legacy/screen_name")
        .and_then(Value::as_str)
        .unwrap_or("i");
    let tweet_url = format!("https://x.com/{screen_name}/status/{tweet_id}");

    // `extended_entities` carries all attachments; `entities` is the fallback.
    let media = legacy
        .pointer("/extended_entities/media")
        .or_else(|| legacy.pointer("/entities/media"))
        .and_then(Value::as_array);
    let media = match media {
        Some(m) => m,
        None => return,
    };

    for m in media {
        if let Some(item) = build_media_item(m, &tweet_id, &tweet_url) {
            out.push(item);
        }
    }
}

fn build_media_item(m: &Value, tweet_id: &str, tweet_url: &str) -> Option<MediaItem> {
    let media_type = m.get("type").and_then(Value::as_str)?.to_string();
    let thumb = m.get("media_url_https").and_then(Value::as_str)?.to_string();
    let id = m
        .get("media_key")
        .or_else(|| m.get("id_str"))
        .and_then(Value::as_str)
        .unwrap_or(tweet_id)
        .to_string();

    let (width, height) = m
        .pointer("/original_info/width")
        .and_then(Value::as_u64)
        .zip(m.pointer("/original_info/height").and_then(Value::as_u64))
        .unwrap_or((0, 0));

    let download_url = match media_type.as_str() {
        "video" | "animated_gif" => best_video_variant(m).unwrap_or_else(|| thumb.clone()),
        _ => format!("{thumb}?name=orig"),
    };

    Some(MediaItem {
        id,
        tweet_id: tweet_id.to_string(),
        media_type,
        thumbnail_url: thumb,
        download_url,
        tweet_url: tweet_url.to_string(),
        width,
        height,
    })
}

/// Pick the highest-bitrate mp4 variant from a video/gif's `video_info`.
fn best_video_variant(m: &Value) -> Option<String> {
    let variants = m.pointer("/video_info/variants")?.as_array()?;
    variants
        .iter()
        .filter(|v| v.get("content_type").and_then(Value::as_str) == Some("video/mp4"))
        .max_by_key(|v| v.get("bitrate").and_then(Value::as_u64).unwrap_or(0))
        .and_then(|v| v.get("url").and_then(Value::as_str))
        .map(str::to_string)
}

/// The big `features` flag blob UserMedia requires. Rotates; see module note.
fn media_features() -> Value {
    json!({
        "rweb_tipjar_consumption_enabled": true,
        "responsive_web_graphql_exclude_directive_enabled": true,
        "verified_phone_label_enabled": false,
        "creator_subscriptions_tweet_preview_api_enabled": true,
        "responsive_web_graphql_timeline_navigation_enabled": true,
        "responsive_web_graphql_skip_user_profile_image_extensions_enabled": false,
        "communities_web_enable_tweet_community_results_fetch": true,
        "c9s_tweet_anatomy_moderator_badge_enabled": true,
        "articles_preview_enabled": true,
        "responsive_web_edit_tweet_api_enabled": true,
        "graphql_is_translatable_rweb_tweet_is_translatable_enabled": true,
        "view_counts_everywhere_api_enabled": true,
        "longform_notetweets_consumption_enabled": true,
        "responsive_web_twitter_article_tweet_consumption_enabled": true,
        "tweet_awards_web_tipping_enabled": false,
        "creator_subscriptions_quote_tweet_preview_enabled": false,
        "freedom_of_speech_not_reach_fetch_enabled": true,
        "standardized_nudges_misinfo": true,
        "tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled": true,
        "rweb_video_timestamps_enabled": true,
        "longform_notetweets_rich_text_read_enabled": true,
        "longform_notetweets_inline_media_enabled": true,
        "responsive_web_enhance_cards_enabled": false
    })
}
