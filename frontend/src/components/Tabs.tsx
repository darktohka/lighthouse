import type { ReactNode } from 'react'

import { cx } from '../lib/cx'

export type TabItem = { id: string; label: string }

export type TabNavProps = {
  tabs: readonly TabItem[]
  active: string
  onChange: (id: string) => void
  label: string
  className?: string
}

function tabClass(active: boolean): string {
  return cx(
    '-mb-px border-b-2 px-3 py-1.5 text-sm font-medium',
    active
      ? 'border-accent text-foreground'
      : 'border-transparent text-muted hover:text-foreground',
  )
}

export function TabNav({ tabs, active, onChange, label, className }: TabNavProps) {
  const focusTab = (index: number) => {
    const next = tabs[index]
    if (!next) return
    onChange(next.id)
    const element = document.getElementById(`tab-${next.id}`)
    if (element instanceof HTMLElement) element.focus()
  }

  return (
    <div
      role="tablist"
      aria-label={label}
      className={cx('flex flex-wrap gap-1 border-b border-border', className)}
    >
      {tabs.map((tab, index) => (
        <button
          key={tab.id}
          type="button"
          role="tab"
          id={`tab-${tab.id}`}
          aria-selected={active === tab.id}
          aria-controls={`panel-${tab.id}`}
          tabIndex={active === tab.id ? 0 : -1}
          onClick={() => onChange(tab.id)}
          onKeyDown={(event) => {
            if (event.key === 'ArrowRight') {
              event.preventDefault()
              focusTab((index + 1) % tabs.length)
            } else if (event.key === 'ArrowLeft') {
              event.preventDefault()
              focusTab((index - 1 + tabs.length) % tabs.length)
            } else if (event.key === 'Home') {
              event.preventDefault()
              focusTab(0)
            } else if (event.key === 'End') {
              event.preventDefault()
              focusTab(tabs.length - 1)
            }
          }}
          className={tabClass(active === tab.id)}
        >
          {tab.label}
        </button>
      ))}
    </div>
  )
}

export type TabPanelProps = {
  id: string
  active: string
  children: ReactNode
  className?: string
}

export function TabPanel({ id, active, children, className }: TabPanelProps) {
  if (id !== active) return null
  return (
    <div
      role="tabpanel"
      id={`panel-${id}`}
      aria-labelledby={`tab-${id}`}
      tabIndex={0}
      className={cx('pt-4', className)}
    >
      {children}
    </div>
  )
}
