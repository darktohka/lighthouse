import { MoonIcon, SunIcon } from '@primer/octicons-react'

import { useTheme } from '../lib/theme-context'
import { Button } from './primitives/Button'

export function ThemeToggle() {
  const { theme, toggleTheme } = useTheme()
  const next = theme === 'dark' ? 'light' : 'dark'
  return (
    <Button
      size="sm"
      onClick={toggleTheme}
      aria-label={`Switch to ${next} theme`}
      title={`Switch to ${next} theme`}
      className="w-7 !px-0"
    >
      {theme === 'dark' ? (
        <SunIcon size={16} aria-hidden="true" />
      ) : (
        <MoonIcon size={16} aria-hidden="true" />
      )}
    </Button>
  )
}
