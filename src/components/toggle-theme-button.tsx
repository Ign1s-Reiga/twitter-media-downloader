'use client';

import { useTheme } from 'next-themes';
import { Button } from '@/components/ui/button';
import { MoonIcon, SunIcon } from 'lucide-react';

export default function ToggleThemeButton() {
  const { resolvedTheme, setTheme } = useTheme();

  const toggleTheme = () => {
    setTheme(resolvedTheme === 'dark' ? 'light' : 'dark');
  };

  return(
    <Button onClick={toggleTheme} className="w-full">
      {resolvedTheme === 'dark'
        ? <MoonIcon className="mr-1" />
        : <SunIcon className="mr-1" />
      }
      Toggle Theme
    </Button>
  );
}
