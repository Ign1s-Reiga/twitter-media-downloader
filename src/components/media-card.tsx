'use client';

import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener';
import { toast } from 'sonner';
import { DownloadIcon, ExternalLinkIcon, FilmIcon, LinkIcon, PlayIcon } from 'lucide-react';
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

/** Append X's image size hint to a pbs.twimg.com URL for lighter previews. */
function sized(url: string, name: 'small' | 'medium' | 'large') {
  const sep = url.includes('?') ? '&' : '?';
  return `${url}${sep}name=${name}`;
}

export function MediaCard({ item }: { item: MediaItem }) {
  const isVideo = item.media_type === 'video';
  const isGif = item.media_type === 'animated_gif';
  const isPlayable = isVideo || isGif;
  const kind = isGif ? 'GIF' : isVideo ? 'Video' : 'Image';
  const [downloading, setDownloading] = useState(false);

  const download = async () => {
    setDownloading(true);
    try {
      const ext = item.media_type === 'photo' ? 'jpg' : 'mp4';
      const filename = `${item.tweet_id || 'media'}_${item.id}.${ext}`;
      const path = await invoke<string>('download_media', { url: item.download_url, filename });
      toast.success('Download complete', {
        description: path,
        action: { label: 'Show', onClick: () => void revealItemInDir(path).catch(() => {}) },
      });
    } catch (err) {
      toast.error(typeof err === 'string' ? err : 'Download failed.');
    } finally {
      setDownloading(false);
    }
  };

  const copyLink = async () => {
    try {
      await navigator.clipboard.writeText(item.tweet_url);
      toast.success('Post link copied to clipboard.');
    } catch {
      // Clipboard API can be unavailable; show the link so it can be copied by hand.
      toast.info('Copy this link', { description: item.tweet_url });
    }
  };

  return (
    <Dialog>
      <DialogTrigger asChild>
        <button
          type="button"
          title="Preview"
          className="group relative block overflow-hidden rounded-lg border border-border bg-muted/40 focus-visible:ring-3 focus-visible:ring-ring/30 focus-visible:outline-none"
        >
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
              src={item.download_url}
              poster={item.thumbnail_url}
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
            <Button variant="outline" onClick={() => void openUrl(item.tweet_url)}>
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