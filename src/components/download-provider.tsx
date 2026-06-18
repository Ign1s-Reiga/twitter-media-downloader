'use client';

import { createContext, useCallback, useContext, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { revealItemInDir } from '@tauri-apps/plugin-opener';
import { toast } from 'sonner';
import { Semaphore } from '@/lib/semaphore';
import { MediaItem, mediaFilename } from '@/lib/media';

export interface DownloadTask {
  id: string;
  label: string;
  total: number;
  done: number;
  failed: number;
  status: 'running' | 'completed';
}

interface DownloadContextValue {
  tasks: DownloadTask[];
  /** Queue a download order; resolves when the whole order finishes. */
  enqueue: (items: MediaItem[], label: string) => Promise<void>;
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

  const enqueue = useCallback(async (items: MediaItem[], label: string) => {
    if (items.length === 0) return;

    const id = newId();
    setTasks((prev) => [
      { id, label, total: items.length, done: 0, failed: 0, status: 'running' },
      ...prev,
    ]);

    let done = 0;
    let failed = 0;
    let firstPath: string | undefined;

    await Promise.all(
      items.map((item) =>
        semaphore.run(async () => {
          try {
            const path = await invoke<string>('download_media', {
              url: item.download_url,
              filename: mediaFilename(item),
            });
            if (!firstPath) firstPath = path;
            done += 1;
          } catch {
            failed += 1;
          }
          setTasks((prev) => prev.map((t) => (t.id === id ? { ...t, done, failed } : t)));
        }),
      ),
    );

    setTasks((prev) => prev.map((t) => (t.id === id ? { ...t, status: 'completed' } : t)));

    // A single toast, only once the entire order has finished.
    const savedPath = firstPath;
    const action = savedPath
      ? { label: 'Show', onClick: () => void revealItemInDir(savedPath).catch(() => {}) }
      : undefined;
    if (failed === 0) {
      toast.success(`${label} — downloaded ${done} item${done === 1 ? '' : 's'}.`, { action });
    } else {
      toast.warning(`${label} — ${done} done, ${failed} failed.`, { action });
    }
  }, []);

  return <DownloadContext.Provider value={{ tasks, enqueue }}>{children}</DownloadContext.Provider>;
}

export function useDownloads(): DownloadContextValue {
  const ctx = useContext(DownloadContext);
  if (!ctx) {
    throw new Error('useDownloads must be used within DownloadProvider');
  }
  return ctx;
}