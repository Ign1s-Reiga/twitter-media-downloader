//! Platform-neutral media model shared by every scraper. The frontend renders
//! and downloads these without caring which site they came from.

use serde::Serialize;

/// A single media attachment ready for the frontend to preview / download.
#[derive(Debug, Serialize)]
pub struct MediaItem {
    /// Stable media key from the platform (used as a React key / dedupe key).
    pub id: String,
    /// Id of the post the media belongs to (tweet id / Instagram shortcode).
    pub post_id: String,
    /// "photo" | "video" | "animated_gif".
    pub media_type: String,
    /// Smaller URL suitable for grid previews.
    pub thumbnail_url: String,
    /// Full-resolution image, or the best video/gif variant to download.
    pub download_url: String,
    /// Link to the post on the platform's website.
    pub post_url: String,
    pub width: u64,
    pub height: u64,
}

/// One page of results plus the cursor to fetch the next page.
#[derive(Debug, Serialize)]
pub struct MediaPage {
    pub items: Vec<MediaItem>,
    pub next_cursor: Option<String>,
}
