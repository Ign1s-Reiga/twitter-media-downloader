'use client';

import { Field, FieldDescription, FieldGroup, FieldLabel } from '@/components/ui/field';
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from '@/components/ui/input-group';
import { CheckIcon, DownloadIcon, FolderIcon, GlobeIcon, ImageIcon, KeyRoundIcon, ListChecksIcon, MaximizeIcon, MinimizeIcon, SearchIcon, UserRoundIcon, VideoIcon, XIcon } from 'lucide-react';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import { Kbd } from '@/components/ui/kbd';
import ToggleThemeButton from '@/components/toggle-theme-button';
import { useEffect, useState, KeyboardEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { ScrollArea } from '@/components/ui/scroll-area';
import { Skeleton } from '@/components/ui/skeleton';
import { MediaCard } from '@/components/media-card';
import { MediaItem } from '@/lib/media';
import { Spinner } from '@/components/ui/spinner';
import { useDownloads, type DownloadStatus } from '@/components/download-provider';

interface MediaResponse {
  items: MediaItem[];
  next_cursor: string | null;
}

interface BrowserInfo {
  id: string;
  label: string;
}

// Account source is either 'guest', 'manual', or a detected browser id.
const GUEST = 'guest';
const MANUAL = 'manual';

function TaskStatusIcon({ status }: { status: DownloadStatus }) {
  switch (status) {
    case 'downloading':
      return <Spinner className="text-primary" />;
    case 'completed':
      return <CheckIcon className="size-4 text-primary" />;
    case 'failed':
      return <XIcon className="size-4 text-destructive" />;
    default:
      return <span className="size-2 rounded-full bg-muted-foreground/40" aria-hidden />;
  }
}

export default function Page() {
  const [open, setOpen] = useState(false);
  const [accountUrl, setAccountUrl] = useState('');
  const [filters, setFilters] = useState<string[]>([]);
  const [sessionToken, setSessionToken] = useState('');
  const [accountSource, setAccountSource] = useState(GUEST);
  const [browsers, setBrowsers] = useState<BrowserInfo[]>([]);
  const [items, setItems] = useState<MediaItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [searched, setSearched] = useState(false);
  const [selectMode, setSelectMode] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const { tasks, enqueue, reset } = useDownloads();

  // Detect which browsers we can import the logged-in X account from.
  useEffect(() => {
    invoke<BrowserInfo[]>('list_browsers')
      .then(setBrowsers)
      .catch(() => setBrowsers([]));
  }, []);

  const retrieve = async () => {
    if (loading) return;
    if (!accountUrl.trim()) {
      toast.warning('Enter an X/Twitter account URL or @handle first.');
      return;
    }

    setLoading(true);
    setSearched(true);
    setSelectMode(false);
    setSelected(new Set());
    try {
      const usingBrowser = accountSource !== GUEST && accountSource !== MANUAL;
      const res = await invoke<MediaResponse>('fetch_user_media', {
        accountUrl,
        filters,
        browser: usingBrowser ? accountSource : null,
        sessionToken: accountSource === MANUAL ? sessionToken.trim() || null : null,
      });
      setItems(res.items);
      reset(); // new results arrived: clear finished task history (keeps any in-flight)
      if (res.items.length === 0) {
        toast.info('No media found for that account with the current filters.');
      } else {
        toast.success(`Found ${res.items.length} media item${res.items.length === 1 ? '' : 's'}.`);
      }
    } catch (err) {
      setItems([]);
      toast.error(typeof err === 'string' ? err : 'Failed to retrieve media.');
    } finally {
      setLoading(false);
    }
  };

  const onAccountKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      retrieve();
    }
  };

  const toggleSelected = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  };

  const allSelected = items.length > 0 && selected.size === items.length;
  const toggleSelectAll = () => {
    setSelected(allSelected ? new Set() : new Set(items.map((i) => i.id)));
  };

  const exitSelectMode = () => {
    setSelectMode(false);
    setSelected(new Set());
  };

  const selectedItems = items.filter((i) => selected.has(i.id));

  return (
    <div className="grid grid-cols-[1fr_360px] h-svh">
      <div className="flex w-full h-full flex-col gap-4 p-6 text-sm leading-loose">
        <Card className="w-full">
          <CardHeader>
            <CardTitle>X/Twitter Media Downloader</CardTitle>
          </CardHeader>
          <CardContent>
            <Collapsible open={open} onOpenChange={setOpen} className="flex items-start gap-2">
              <FieldGroup>
                <Field>
                  <FieldLabel htmlFor="account-url">Account URL</FieldLabel>
                  <InputGroup id="account-url">
                    <InputGroupInput
                      placeholder="Retrive..."
                      value={accountUrl}
                      onChange={(e) => setAccountUrl(e.target.value)}
                      onKeyDown={onAccountKeyDown}
                      disabled={loading}
                    />
                    <InputGroupAddon>
                      <SearchIcon className="text-muted-foreground" />
                    </InputGroupAddon>
                    <InputGroupAddon align="inline-end">
                      <Kbd>Enter</Kbd>
                    </InputGroupAddon>
                  </InputGroup>
                  <FieldDescription>The URL of the X/Twitter account you want to get posts from.</FieldDescription>
                </Field>
                <CollapsibleContent className="grid grid-cols-2 gap-x-2 gap-y-4">
                  <Field className="col-span-2">
                    <FieldLabel htmlFor="account-source">Account</FieldLabel>
                    <ToggleGroup
                      type="single"
                      id="account-source"
                      className="w-full flex-wrap"
                      variant="outline"
                      value={accountSource}
                      onValueChange={(v) => v && setAccountSource(v)}
                    >
                      <ToggleGroupItem value={GUEST} className="grow">
                        <UserRoundIcon className="mr-1" />
                        Guest
                      </ToggleGroupItem>
                      {browsers.map((b) => (
                        <ToggleGroupItem key={b.id} value={b.id} className="grow">
                          <GlobeIcon className="mr-1" />
                          {b.label}
                        </ToggleGroupItem>
                      ))}
                      <ToggleGroupItem value={MANUAL} className="grow">
                        <KeyRoundIcon className="mr-1" />
                        Manual
                      </ToggleGroupItem>
                    </ToggleGroup>
                    <FieldDescription>
                      Import the logged-in account from a browser, or paste a token manually. Guest mode works only for public accounts and is heavily rate-limited by X.
                    </FieldDescription>
                  </Field>
                  <Field>
                    <FieldLabel htmlFor="media-filter">Media content filter</FieldLabel>
                    <ToggleGroup
                      type="multiple"
                      id="media-filter"
                      className="w-full"
                      variant="outline"
                      value={filters}
                      onValueChange={setFilters}
                    >
                      <ToggleGroupItem value="images" className="grow">
                        <ImageIcon className="mr-1" />
                        Images
                      </ToggleGroupItem>
                      <ToggleGroupItem value="videos" className="grow">
                        <VideoIcon className="mr-1" />
                        Videos
                      </ToggleGroupItem>
                    </ToggleGroup>
                    <FieldDescription>Filter the media content you want to retrieve.</FieldDescription>
                  </Field>
                  <Field>
                    <FieldLabel htmlFor="download-location">Download Location</FieldLabel>
                    <InputGroup id="download-location">
                      <InputGroupInput placeholder="Select download location..." />
                      <InputGroupButton>
                        <FolderIcon />
                      </InputGroupButton>
                    </InputGroup>
                    <FieldDescription>The location where you want to save the downloaded media.</FieldDescription>
                  </Field>
                  {accountSource === MANUAL && (
                    <Field className="col-span-2">
                      <FieldLabel htmlFor="session-token">Session Token</FieldLabel>
                      <InputGroup id="session-token">
                        <InputGroupInput
                          placeholder="auth_token value, or auth_token=…; ct0=…"
                          type="password"
                          value={sessionToken}
                          onChange={(e) => setSessionToken(e.target.value)}
                        />
                      </InputGroup>
                      <FieldDescription>
                        Paste your X <code>auth_token</code> cookie (optionally with <code>ct0</code>). Used to scrape as your account when browser import isn't available.
                      </FieldDescription>
                    </Field>
                  )}
                </CollapsibleContent>
              </FieldGroup>
              <CollapsibleTrigger asChild>
                <Button variant="outline" size="icon">
                  {open ? <MinimizeIcon /> : <MaximizeIcon />}
                </Button>
              </CollapsibleTrigger>
            </Collapsible>
          </CardContent>
        </Card>
        <ScrollArea className="grow h-0 pr-2">
          {loading ? (
            <div className="grid grid-cols-[repeat(auto-fill,minmax(200px,1fr))] gap-4">
              {[...Array(12)].map((_, i) => (
                <Skeleton key={i} className="aspect-square rounded-lg" />
              ))}
            </div>
          ) : items.length > 0 ? (
            <div className="grid grid-cols-[repeat(auto-fill,minmax(200px,1fr))] gap-4">
              {items.map((item) =>
                selectMode ? (
                  <button
                    key={item.id}
                    type="button"
                    onClick={() => toggleSelected(item.id)}
                    aria-pressed={selected.has(item.id)}
                    title={selected.has(item.id) ? 'Deselect' : 'Select'}
                    className="block w-full rounded-lg text-left focus-visible:ring-3 focus-visible:ring-ring/30 focus-visible:outline-none"
                  >
                    <MediaCard item={item} selectMode selected={selected.has(item.id)} />
                  </button>
                ) : (
                  <MediaCard key={item.id} item={item} />
                ),
              )}
            </div>
          ) : (
            <div className="flex h-full min-h-40 items-center justify-center text-center text-muted-foreground">
              {searched
                ? 'No media to show. Try a different account or filter.'
                : 'Enter an account above and press Enter to retrieve its media.'}
            </div>
          )}
        </ScrollArea>
        {selectMode ? (
          <div className="flex items-center gap-2">
            <Button variant="outline" onClick={exitSelectMode}>
              <XIcon className="mr-1" />
              Cancel
            </Button>
            <Button variant="outline" onClick={toggleSelectAll} disabled={items.length === 0}>
              {allSelected ? 'Clear' : 'Select all'}
            </Button>
            <Button
              className="flex-1"
              onClick={() => void enqueue(selectedItems, 'Selected')}
              disabled={selected.size === 0}
            >
              <DownloadIcon className="mr-1" />
              {`Download selected (${selected.size})`}
            </Button>
          </div>
        ) : (
          <div className="flex items-center gap-2">
            <Button variant="outline" onClick={() => setSelectMode(true)} disabled={items.length === 0}>
              <ListChecksIcon className="mr-1" />
              Select
            </Button>
            <Button
              className="flex-1"
              onClick={() => void enqueue(items, 'All media')}
              disabled={items.length === 0}
            >
              <DownloadIcon className="mr-1" />
              {`Download all${items.length > 0 ? ` (${items.length})` : ''}`}
            </Button>
          </div>
        )}
      </div>
      <aside className="border-border border-l flex flex-col h-full">
        <div className="grow flex flex-col min-h-0 p-4">
          <h1 className="text-4xl font-extrabold tracking-tight text-balance">
            Tasks
          </h1>
          <div className="mt-4 grow min-h-0 overflow-y-auto">
            {tasks.length === 0 ? (
              <p className="text-sm text-muted-foreground">No downloads yet.</p>
            ) : (
              <ul className="space-y-2">
                {tasks.map((t) => (
                  <li
                    key={t.id}
                    className="flex items-center gap-2 rounded-lg border border-border p-2.5 text-sm"
                  >
                    <span className="flex size-4 shrink-0 items-center justify-center">
                      <TaskStatusIcon status={t.status} />
                    </span>
                    <span className="min-w-0 flex-1 truncate" title={t.label}>
                      {t.label}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>
        <div className="border-t border-border p-4 flex items-center">
          <ToggleThemeButton />
        </div>
      </aside>
    </div>
  );
}