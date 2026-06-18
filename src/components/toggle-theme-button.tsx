'use client';

import { useEffect, useState } from 'react';
import { useTheme } from 'next-themes';
import { Button } from '@/components/ui/button';
import { MoonIcon, SunIcon } from 'lucide-react';

export default function ToggleThemeButton() {
  const { resolvedTheme, setTheme } = useTheme();
  // `resolvedTheme` is undefined during SSR / the first client render. Gate the
  // theme-dependent icon behind a mount flag so the initial client render matches
  // the server and we avoid a hydration mismatch (next-themes' documented pattern).
  const [mounted, setMounted] = useState(false);
  useEffect(() => setMounted(true), []);

  const toggleTheme = () => {
    setTheme(resolvedTheme === 'dark' ? 'light' : 'dark');
  };

  return(
    <Button onClick={toggleTheme} className="w-full">
      {mounted && resolvedTheme === 'dark'
        ? <MoonIcon className="mr-1" />
        : <SunIcon className="mr-1" />
      }
      Toggle Theme
    </Button>
  );
}
