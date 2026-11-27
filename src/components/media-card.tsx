'use client';

import { useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener';
import { toast } from 'sonner';
import { CameraIcon, CheckIcon, DownloadIcon, ExternalLinkIcon, FilmIcon, LinkIcon, PlayIcon } from 'lucide-react';
import { AspectRatio } from '@/components/ui/aspect-ratio';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';
import { cn } from '@/lib/utils';
import { MediaItem, frameFilename, mediaFilename } from '@/lib/media';
import { useDownloads } from '@/components/download-provider';

/**
 * Append X's image size hint to a pbs.twimg.com URL for lighter previews. Other
 * CDNs are left alone: Instagram's URLs are signed, and the scraper already
 * picks a grid-sized rendition for them.
 */
function sized(url: string, name: 'small' | 'medium' | 'large') {
  if (!url.includes('pbs.twimg.com/')) return url;
  const sep = url.includes('?') ? '&' : '?';
  return `${url}${sep}name=${name}`;
}

/**
 * Route video through the Rust `twmedia` proxy so the webview can play it: some
 * CDNs (video.twimg.com) refuse cross-origin requests that lack their Referer,
 * which a `<video>` element can't set. The proxy fetches it server-side and
 * streams it back same-origin. (Custom protocols are served at
 * `http://<scheme>.localhost`.)
 */
function proxied(url: string) {
  return `http://twmedia.localhost/?u=${encodeURIComponent(url)}`;
}

export function MediaCard({
  item,
  selectMode = false,
  selected = false,
}: {
  item: MediaItem;
  selectMode?: boolean;
  selected?: boolean;
}) {
  const isVideo = item.media_type === 'video';
  const isGif = item.media_type === 'animated_gif';
  const isPlayable = isVideo || isGif;
  const kind = isGif ? 'GIF' : isVideo ? 'Video' : 'Image';
  const { enqueue, downloadDir, downloaded } = useDownloads();
  const isDownloaded = downloaded.has(mediaFilename(item));
  const [downloading, setDownloading] = useState(false);
  const [capturing, setCapturing] = useState(false);
  const videoRef = useRef<HTMLVideoElement>(null);

  const download = async () => {
    setDownloading(true);
    try {
      await enqueue([item], kind);
    } finally {
      setDownloading(false);
    }
  };

  /**
   * Save the frame the video is showing right now as a PNG. A playing video is
   * paused first so what's on screen is exactly what gets saved (GIFs have no
   * controls to resume with, so they keep looping).
   */
  const captureFrame = async () => {
    const video = videoRef.current;
    if (!video) return;
    if (video.readyState < HTMLMediaElement.HAVE_CURRENT_DATA) {
      toast.warning('The video has not loaded a frame yet.');
      return;
    }
    if (isVideo && !video.paused) video.pause();

    setCapturing(true);
    try {
      const canvas = document.createElement('canvas');
      canvas.width = video.videoWidth;
      canvas.height = video.videoHeight;
      canvas.getContext('2d')?.drawImage(video, 0, 0);
      // toBlob throws a SecurityError here if the video tainted the canvas
      // (i.e. the proxy stopped sending CORS headers); the catch below reports it.
      const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, 'image/png'));
      if (!blob) throw new Error('Could not encode the frame.');

      const filename = frameFilename(item, video.currentTime);
      // Raw bytes as the IPC body; the paths go in headers, percent-encoded
      // because header values can't carry non-ASCII folder names.
      const path = await invoke<string>('save_frame', new Uint8Array(await blob.arrayBuffer()), {
        headers: {
          'x-filename': encodeURIComponent(filename),
          'x-dir': encodeURIComponent(downloadDir ?? ''),
        },
      });
      toast.success('Frame saved.', {
        description: filename,
        action: { label: 'Show', onClick: () => void revealItemInDir(path).catch(() => {}) },
      });
    } catch (err) {
      toast.error('Failed to capture the frame.', {
        description: typeof err === 'string' ? err : String(err),
      });
    } finally {
      setCapturing(false);
    }
  };

  const copyLink = async () => {
    try {
      await navigator.clipboard.writeText(item.post_url);
      toast.success('Post link copied to clipboard.');
    } catch {
      // Clipboard API can be unavailable; show the link so it can be copied by hand.
      toast.info('Copy this link', { description: item.post_url });
    }
  };

  const thumbnail = (
    <>
      <AspectRatio ratio={1}>
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img
          src={sized(item.thumbnail_url, 'small')}
          alt=""
          loading="lazy"
          className="h-full w-full object-cover transition-transform duration-200 group-hover:scale-105"
        />
      </AspectRatio>

      {isPlayable && (
        <span className="absolute left-2 top-2 flex items-center gap-1 rounded-md bg-black/65 px-1.5 py-0.5 text-xs font-medium text-white backdrop-blur-sm">
          {isGif ? <FilmIcon className="size-3" /> : <PlayIcon className="size-3" />}
          {kind}
        </span>
      )}

      {isDownloaded && (
        <span className="absolute bottom-2 left-2 flex items-center gap-1 rounded-md bg-primary px-1.5 py-0.5 text-xs font-medium text-primary-foreground shadow-sm">
          <CheckIcon className="size-3" />
          Downloaded
        </span>
      )}
    </>
  );

  // Select mode: render a non-interactive visual; the parent wraps this in the
  // clickable control that toggles selection. The top-right checkbox is a custom
  // visual (the shared Checkbox renders a <button>, which can't nest in a button).
  if (selectMode) {
    return (
      <div
        className={cn(
          'group relative block overflow-hidden rounded-lg border bg-muted/40',
          selected ? 'border-primary ring-2 ring-primary' : 'border-border',
        )}
      >
        {thumbnail}
        {selected && <span className="pointer-events-none absolute inset-0 bg-primary/20" />}
        <span
          aria-hidden
          className={cn(
            'absolute right-2 top-2 flex size-5 items-center justify-center rounded-[5px] border shadow-sm',
            selected ? 'border-primary bg-primary text-primary-foreground' : 'border-input bg-background/80',
          )}
        >
          {selected && <CheckIcon className="size-3.5" />}
        </span>
      </div>
    );
  }

  // Normal mode: click opens the preview dialog.
  return (
    <Dialog>
      <DialogTrigger asChild>
        <button
          type="button"
          title="Preview"
          className="group relative block overflow-hidden rounded-lg border border-border bg-muted/40 focus-visible:ring-3 focus-visible:ring-ring/30 focus-visible:outline-none"
        >
          {thumbnail}
        </button>
      </DialogTrigger>

      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle>{kind} preview</DialogTitle>
          <DialogDescription>
            {item.width > 0 && item.height > 0 ? `${item.width} × ${item.height}` : 'Media from the selected post.'}
          </DialogDescription>
        </DialogHeader>

        <div className="flex max-h-[70vh] items-center justify-center overflow-hidden rounded-lg bg-muted/40">
          {isPlayable ? (
            <video
              ref={videoRef}
              src={proxied(item.download_url)}
              // crossOrigin makes the poster a CORS fetch as well, and the CDNs
              // don't reliably allow that — so it goes through the proxy too.
              poster={proxied(item.thumbnail_url)}
              crossOrigin="anonymous"
              controls={isVideo}
              autoPlay={isGif}
              loop={isGif}
              muted={isGif}
              playsInline
              className="max-h-[70vh] w-auto"
            />
          ) : (
            // eslint-disable-next-line @next/next/no-img-element
            <img src={item.download_url} alt="" className="max-h-[70vh] w-auto object-contain" />
          )}
        </div>

        <DialogFooter className="sm:justify-between">
          <Button variant="outline" onClick={copyLink}>
            <LinkIcon className="mr-1" />
            Copy link
          </Button>
          <div className="flex gap-2">
            {isPlayable && (
              <Button variant="outline" onClick={captureFrame} disabled={capturing}>
                <CameraIcon className="mr-1" />
                {capturing ? 'Capturing…' : 'Capture frame'}
              </Button>
            )}
            <Button variant="outline" onClick={() => void openUrl(item.post_url)}>
              <ExternalLinkIcon className="mr-1" />
              Open post
            </Button>
            <Button onClick={download} disabled={downloading}>
              <DownloadIcon className="mr-1" />
              {downloading ? 'Downloading…' : 'Download'}
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export default MediaCard;