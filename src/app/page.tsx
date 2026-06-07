'use client';

import { Field, FieldDescription, FieldGroup, FieldLabel } from '@/components/ui/field';
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from '@/components/ui/input-group';
import { FolderIcon, ImageIcon, MaximizeIcon, MinimizeIcon, SearchIcon, VideoIcon } from 'lucide-react';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import { Kbd } from '@/components/ui/kbd';
import ToggleThemeButton from '@/components/toggle-theme-button';
import { JSX, useState, KeyboardEvent } from 'react';
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { ScrollArea } from '@/components/ui/scroll-area';
import { Skeleton } from '@/components/ui/skeleton';

export default function Page() {
  const [open, setOpen] = useState(false);
  const [result, setResult] = useState<JSX.Element[]>([]);

  const retrieve = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === 'Enter') {
      setResult([...Array(10)].map((_, i) => <Skeleton key={i} className="aspect-square" />));
    }
  }

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
                    <InputGroupInput placeholder="Retrive..." onKeyDown={(e) => retrieve(e)} />
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
                  <Field>
                    <FieldLabel htmlFor="media-filter">Media content filter</FieldLabel>
                    <ToggleGroup type="multiple" id="media-filter" className="w-full" variant="outline">
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
                  <Field>
                    <FieldLabel htmlFor="session-token">Session Token</FieldLabel>
                    <InputGroup id="session-token">
                      <InputGroupInput placeholder="Enter your X/Twitter session token..." type="password" />
                    </InputGroup>
                    <FieldDescription>
                      The session token for your X/Twitter account. It's required for downloading specific content. In most cases, you don't need this.
                    </FieldDescription>
                  </Field>
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
          <div className="grid grid-cols-[repeat(auto-fill,minmax(200px,1fr))] gap-4">
            {...result}
          </div>
        </ScrollArea>
        <Button className="w-full">Download</Button>
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
