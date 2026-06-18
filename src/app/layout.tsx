import { Geist_Mono, Noto_Sans, Inter } from 'next/font/google';

import './globals.css';
import { ThemeProvider } from '@/components/theme-provider';
import { DownloadProvider } from '@/components/download-provider';
import { Toaster } from '@/components/ui/sonner';
import { cn } from '@/lib/utils';

const interHeading = Inter({subsets:['latin'],variable:'--font-heading'});
const notoSans = Noto_Sans({subsets:['latin'],variable:'--font-sans'});
const fontMono = Geist_Mono({
  subsets: ['latin'],
  variable: '--font-mono',
});

export default function RootLayout({ children }: Readonly<{
  children: React.ReactNode
}>) {
  return (
    <html
      lang="en"
      suppressHydrationWarning
      className={cn('antialiased', fontMono.variable, 'font-sans', "font-sans", notoSans.variable, interHeading.variable)}
    >
      <body>
        <ThemeProvider>
          <DownloadProvider>
            {children}
          </DownloadProvider>
          <Toaster />
        </ThemeProvider>
      </body>
    </html>
  );
}
