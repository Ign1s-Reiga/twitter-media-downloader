'use client';

import { Field, FieldDescription, FieldGroup, FieldLabel } from '@/components/ui/field';
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from '@/components/ui/input-group';
import { FolderIcon, GlobeIcon, ImageIcon, KeyRoundIcon, MaximizeIcon, MinimizeIcon, SearchIcon, UserRoundIcon, VideoIcon } from 'lucide-react';
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
import { MediaCard, MediaItem } from '@/components/media-card';

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
    try {
      const usingBrowser = accountSource !== GUEST && accountSource !== MANUAL;
      const res = await invoke<MediaResponse>('fetch_user_media', {
        accountUrl,
        filters,
        browser: usingBrowser ? accountSource : null,
        sessionToken: accountSource === MANUAL ? sessionToken.trim() || null : null,
      });
      setItems(res.items);
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
              {items.map((item) => (
                <MediaCard key={item.id} item={item} />
              ))}
            </div>
          ) : (
            <div className="flex h-full min-h-40 items-center justify-center text-center text-muted-foreground">
              {searched
                ? 'No media to show. Try a different account or filter.'
                : 'Enter an account above and press Enter to retrieve its media.'}
            </div>
          )}
        </ScrollArea>
        <Button className="w-full" disabled={items.length === 0}>
          Download
        </Button>
      </div>
      <aside className="border-border border-l flex flex-col h-full">
        <div className="grow p-4">
          <h1 className="text-4xl font-extrabold tracking-tight text-balance">
            Tasks
          </h1>
          <div className="overflow-y-auto">

          </div>
        </div>
        <div className="border-t border-border p-4 flex items-center">
          <ToggleThemeButton />
        </div>
      </aside>
    </div>
  );
}