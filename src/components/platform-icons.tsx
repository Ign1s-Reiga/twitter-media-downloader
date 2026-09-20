import type { ComponentType, SVGProps } from 'react';
import type { Platform } from '@/lib/platform';

type IconProps = SVGProps<SVGSVGElement>;

/** X logo, drawn to match lucide's 24px grid. */
export function XLogo(props: IconProps) {
  return (
    <svg viewBox="0 0 24 24" width="1em" height="1em" fill="currentColor" aria-hidden {...props}>
      <path d="M18.901 1.153h3.68l-8.04 9.19L24 22.846h-7.406l-5.8-7.584-6.638 7.584H.474l8.6-9.83L0 1.154h7.594l5.243 6.932ZM17.61 20.644h2.039L6.486 3.24H4.298Z" />
    </svg>
  );
}

/** Instagram glyph in lucide's stroke style (the brand icon lucide used to ship). */
export function InstagramLogo(props: IconProps) {
  return (
    <svg
      viewBox="0 0 24 24"
      width="1em"
      height="1em"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
      {...props}
    >
      <rect width="20" height="20" x="2" y="2" rx="5" ry="5" />
      <path d="M16 11.37A4 4 0 1 1 12.63 8 4 4 0 0 1 16 11.37z" />
      <line x1="17.5" x2="17.51" y1="6.5" y2="6.5" />
    </svg>
  );
}

export const PLATFORM_ICONS: Record<Platform, ComponentType<IconProps>> = {
  x: XLogo,
  instagram: InstagramLogo,
};
