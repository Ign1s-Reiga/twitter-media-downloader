'use client';

import { createContext, useCallback, useContext, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { revealItemInDir } from '@tauri-apps/plugin-opener';
import { toast } from 'sonner';
import { Semaphore } from '@/lib/semaphore';
import { MediaItem, mediaFilename } from '@/lib/media';

export type DownloadStatus = 'pending' | 'downloading' | 'completed' | 'failed';

/** One task card per MediaItem download. */
export interface DownloadTask {
  id: string;
  label: string;
  status: DownloadStatus;
}

interface DownloadContextValue {
  tasks: DownloadTask[];
  /** Queue a download order — one task card per item; resolves when all finish. */
  enqueue: (items: MediaItem[], orderLabel: string) => Promise<void>;
  /** Clear the task list (e.g. on a new search). */
  reset: () => void;
  /** Folder downloads are saved to; null means the OS Downloads folder. */
  downloadDir: string | null;
  setDownloadDir: (dir: string | null) => void;
}

const DownloadContext = createContext<DownloadContextValue | null>(null);

const MAX_CONCURRENT = 3;
// One shared semaphore across every order, so total in-flight downloads never
// exceed the cap regardless of how many orders are running.
const semaphore = new Semaphore(MAX_CONCURRENT);

function newId(): string {
  return globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

export function DownloadProvider({ children }: { children: React.ReactNode }) {
  const [tasks, setTasks] = useState<DownloadTask[]>([]);

  // Mirror the chosen dir into a ref so enqueue reads the latest without being
  // re-created (and so concurrent downloads all use the current folder).
  const [downloadDir, setDownloadDirState] = useState<string | null>(null);
  const dirRef = useRef<string | null>(null);
  const setDownloadDir = useCallback((dir: string | null) => {
    dirRef.current = dir;
    setDownloadDirState(dir);
  }, []);

  const setStatus = useCallback((id: string, status: DownloadStatus) => {
    setTasks((prev) => prev.map((t) => (t.id === id ? { ...t, status } : t)));
  }, []);

  const reset = useCallback(() => {
    // Keep still-running downloads visible; only clear finished history. This
    // way a new search doesn't orphan in-flight downloads (their cards stay,
    // and their completion toast still lands against a visible task).
    setTasks((prev) => prev.filter((t) => t.status === 'pending' || t.status === 'downloading'));
  }, []);

  const enqueue = useCallback(
    async (items: MediaItem[], orderLabel: string) => {
      if (items.length === 0) return;

      // Each MediaItem becomes its own task card.
      const entries = items.map((item) => ({ id: newId(), item, label: mediaFilename(item) }));
      setTasks((prev) => [
        ...entries.map((e) => ({ id: e.id, label: e.label, status: 'pending' as DownloadStatus })),
        ...prev,
      ]);

      let done = 0;
      let failed = 0;
      let firstPath: string | undefined;

      await Promise.all(
        entries.map((e) =>
          semaphore.run(async () => {
            setStatus(e.id, 'downloading');
            try {
              const path = await invoke<string>('download_media', {
                url: e.item.download_url,
                filename: e.label,
                dir: dirRef.current,
              });
              if (!firstPath) firstPath = path;
              done += 1;
              setStatus(e.id, 'completed');
            } catch {
              failed += 1;
              setStatus(e.id, 'failed');
            }
          }),
        ),
      );

      // A single toast, only once the entire order has finished.
      const savedPath = firstPath;
      const action = savedPath
        ? { label: 'Show', onClick: () => void revealItemInDir(savedPath).catch(() => {}) }
        : undefined;
      if (failed === 0) {
        toast.success(`${orderLabel} — downloaded ${done} item${done === 1 ? '' : 's'}.`, { action });
      } else {
        toast.warning(`${orderLabel} — ${done} done, ${failed} failed.`, { action });
      }
    },
    [setStatus],
  );

  return (
    <DownloadContext.Provider value={{ tasks, enqueue, reset, downloadDir, setDownloadDir }}>
      {children}
    </DownloadContext.Provider>
  );
}

export function useDownloads(): DownloadContextValue {
  const ctx = useContext(DownloadContext);
  if (!ctx) {
    throw new Error('useDownloads must be used within DownloadProvider');
  }
  return ctx;
}