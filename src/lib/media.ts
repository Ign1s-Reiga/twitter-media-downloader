export interface MediaItem {
  id: string;
  /** Id of the post the media belongs to (tweet id / Instagram shortcode). */
  post_id: string;
  media_type: 'photo' | 'video' | 'animated_gif';
  thumbnail_url: string;
  download_url: string;
  /** Link to the post on the platform's website. */
  post_url: string;
  width: number;
  height: number;
}

/** Stable download filename for a media item (shared by single + bulk download). */
export function mediaFilename(item: MediaItem): string {
  const ext = item.media_type === 'photo' ? 'jpg' : 'mp4';
  return `${item.post_id || 'media'}_${item.id}.${ext}`;
}
