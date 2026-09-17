import { useMemo, useSyncExternalStore } from 'react'

export type ChartColors = {
  accent: string
  success: string
  muted: string
  border: string
  foreground: string
  grid: string
}

function readChartColors(_themeClass: string): ChartColors {
  if (typeof window === 'undefined') {
    return {
      accent: 'currentColor',
      success: 'currentColor',
      muted: 'currentColor',
      border: 'currentColor',
      foreground: 'currentColor',
      grid: 'currentColor',
    }
  }
  const style = getComputedStyle(document.documentElement)
  const read = (name: string) => style.getPropertyValue(name).trim() || 'currentColor'
  return {
    accent: read('--lh-accent-fg'),
    success: read('--lh-success-fg'),
    muted: read('--lh-fg-muted'),
    border: read('--lh-border-default'),
    foreground: read('--lh-fg-default'),
    grid: read('--lh-border-muted'),
  }
}

function subscribeToTheme(callback: () => void): () => void {
  if (typeof document === 'undefined') return () => undefined
  const observer = new MutationObserver(callback)
  observer.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['class'],
  })
  return () => observer.disconnect()
}

function themeSnapshot(): string {
  return typeof document === 'undefined' ? '' : document.documentElement.className
}

export function useChartColors(): ChartColors {
  const themeClass = useSyncExternalStore(
    subscribeToTheme,
    themeSnapshot,
    () => '',
  )
  return useMemo(() => readChartColors(themeClass), [themeClass])
}

export const TOOLTIP_STYLE = {
  background: 'var(--lh-canvas-overlay)',
  border: '1px solid var(--lh-border-default)',
  borderRadius: 6,
  fontSize: 12,
  color: 'var(--lh-fg-default)',
}
