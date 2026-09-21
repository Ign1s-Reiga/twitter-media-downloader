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

/** Filename for a still captured from a video `seconds` in, e.g. `…_00m12s340.png`. */
export function frameFilename(item: MediaItem, seconds: number): string {
  const stem = mediaFilename(item).replace(/\.[^.]+$/, '');
  return `${stem}_${formatTimestamp(seconds)}.png`;
}

function formatTimestamp(seconds: number): string {
  const total = Math.max(0, seconds);
  const whole = Math.floor(total);
  const m = Math.floor(whole / 60);
  const s = whole % 60;
  const ms = Math.round((total - whole) * 1000);
  return `${String(m).padStart(2, '0')}m${String(s).padStart(2, '0')}s${String(ms).padStart(3, '0')}`;
}
