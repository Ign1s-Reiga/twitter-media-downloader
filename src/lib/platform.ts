/** Which site the scraper targets; mirrors `Platform` in the Rust backend. */
export type Platform = 'x' | 'instagram';

export interface PlatformInfo {
  id: Platform;
  label: string;
  /** Example account input. */
  placeholder: string;
  /** The login cookie the manual Session Token field expects. */
  tokenCookie: string;
  /** Companion (CSRF) cookie worth pasting alongside it. */
  extraCookie: string;
  /** Caveat about guest mode on this platform. */
  guestNote: string;
}

export const PLATFORMS: PlatformInfo[] = [
  {
    id: 'x',
    label: 'X/Twitter',
    placeholder: 'https://x.com/username or @username',
    tokenCookie: 'auth_token',
    extraCookie: 'ct0',
    guestNote: 'X no longer serves media timelines to guests, so a logged-in browser or a token is effectively required.',
  },
  {
    id: 'instagram',
    label: 'Instagram',
    placeholder: 'https://www.instagram.com/username/ or @username',
    tokenCookie: 'sessionid',
    extraCookie: 'csrftoken',
    guestNote: 'Instagram refuses almost every guest request, so a logged-in browser or a token is effectively required.',
  },
];

export function platformInfo(id: Platform): PlatformInfo {
  return PLATFORMS.find((p) => p.id === id) ?? PLATFORMS[0];
}

/** The platform a pasted profile URL belongs to, or null for a bare handle. */
export function platformFromInput(input: string): Platform | null {
  const s = input.trim().toLowerCase();
  if (/(^|[/.])instagram\.com\//.test(s)) return 'instagram';
  if (/(^|[/.])(x|twitter)\.com\//.test(s)) return 'x';
  return null;
}
