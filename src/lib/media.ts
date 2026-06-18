export interface MediaItem {
  id: string;
  tweet_id: string;
  media_type: 'photo' | 'video' | 'animated_gif';
  thumbnail_url: string;
  download_url: string;
  tweet_url: string;
  width: number;
  height: number;
}

/** Stable download filename for a media item (shared by single + bulk download). */
export function mediaFilename(item: MediaItem): string {
  const ext = item.media_type === 'photo' ? 'jpg' : 'mp4';
  return `${item.tweet_id || 'media'}_${item.id}.${ext}`;
}