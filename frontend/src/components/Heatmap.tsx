import { ChevronLeftIcon, ChevronRightIcon } from '@primer/octicons-react'

import type { Heatmap as HeatmapData, HeatmapDay } from '../api/schemas'
import { cx } from '../lib/cx'
import { formatNumber } from '../lib/format'
import { Button } from './primitives/Button'
import { ErrorState, LoadingState } from './primitives/StateViews'

const WEEKDAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']
const MONTHS = [
  'Jan',
  'Feb',
  'Mar',
  'Apr',
  'May',
  'Jun',
  'Jul',
  'Aug',
  'Sep',
  'Oct',
  'Nov',
  'Dec',
]

const CELL = 12
const GAP = 3

const LEVEL_CLASSES = [
  'bg-canvas-subtle border border-border',
  'bg-accent/25',
  'bg-accent/45',
  'bg-accent/70',
  'bg-accent',
] as const

type PlacedCell = {
  date: string
  count: number
  level: number
  week: number
  weekday: number
}

function levelFor(count: number, max: number): number {
  if (count <= 0 || max <= 0) return 0
  const ratio = count / max
  if (ratio >= 0.75) return 4
  if (ratio >= 0.5) return 3
  if (ratio >= 0.25) return 2
  return 1
}

function shiftIsoDate(iso: string, days: number): string {
  const base = Date.parse(`${iso}T00:00:00Z`)
  return new Date(base + days * 86_400_000).toISOString().slice(0, 10)
}

function formatDay(iso: string): string {
  const date = new Date(`${iso}T00:00:00Z`)
  if (Number.isNaN(date.getTime())) return iso
  return new Intl.DateTimeFormat('en-US', {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    timeZone: 'UTC',
  }).format(date)
}

function buildGrid(start: string, end: string, days: readonly HeatmapDay[]) {
  const totalDays =
    Math.round(
      (Date.parse(`${end}T00:00:00Z`) - Date.parse(`${start}T00:00:00Z`)) /
        86_400_000,
    ) + 1
  const weeks = totalDays / 7
  const byDate = new Map(days.map((day) => [day.date, day.count]))
  const max = days.reduce((acc, day) => Math.max(acc, day.count), 0)

  const cells: PlacedCell[] = []
  const monthLabels: Array<{ week: number; label: string }> = []
  for (let index = 0; index < totalDays; index += 1) {
    const date = shiftIsoDate(start, index)
    const count = byDate.get(date) ?? 0
    const week = Math.floor(index / 7)
    const weekday = index % 7
    cells.push({
      date,
      count,
      level: levelFor(count, max),
      week,
      weekday,
    })
    if (index === 0 || date.slice(8, 10) === '01') {
      monthLabels.push({
        week,
        label: MONTHS[Number(date.slice(5, 7)) - 1],
      })
    }
  }
  return { weeks, cells, monthLabels }
}

export type HeatmapProps = {
  endDate: string
  data: HeatmapData | null
  loading: boolean
  error: Error | null
  onRetry: () => void
  onWindowChange: (nextEnd: string) => void
  maxEnd: string
}

export function Heatmap({
  endDate,
  data,
  loading,
  error,
  onRetry,
  onWindowChange,
  maxEnd,
}: HeatmapProps) {
  const grid = data ? buildGrid(data.start, data.end, data.days) : null
  const canGoNext = shiftIsoDate(endDate, 364) <= maxEnd

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-sm font-semibold">
          {data
            ? `${formatNumber(data.total)} contributions · ${formatDay(data.start)} – ${formatDay(data.end)}`
            : 'Contributions'}
        </h3>
        <div className="flex items-center gap-1">
          <Button
            size="sm"
            disabled={Number(endDate.slice(0, 4)) <= 2026}
            aria-label="Previous year"
            onClick={() => onWindowChange(shiftIsoDate(endDate, -364))}
            leadingIcon={<ChevronLeftIcon size={14} aria-hidden="true" />}
          >
            {Number(endDate.slice(0, 4)) - 1}
          </Button>
          <Button
            size="sm"
            disabled={!canGoNext}
            aria-label="Next year"
            onClick={() => onWindowChange(shiftIsoDate(endDate, 364))}
            trailingIcon={<ChevronRightIcon size={14} aria-hidden="true" />}
          >
            {Number(endDate.slice(0, 4)) + 1}
          </Button>
        </div>
      </div>

      {loading && !data ? <LoadingState label="Loading contributions…" /> : null}
      {error ? <ErrorState error={error} onRetry={onRetry} /> : null}

      {grid && data ? (
        <>
          <div className="overflow-x-auto pb-1 scrollbar-thin">
            <div
              role="img"
              aria-label={`Contribution heatmap from ${data.start} to ${data.end}`}
              className="grid w-max"
              style={{
                gap: GAP,
                gridTemplateColumns: `auto repeat(${grid.weeks}, ${CELL}px)`,
                gridTemplateRows: `auto repeat(7, ${CELL}px)`,
              }}
            >
              {grid.monthLabels.map((month) => (
                <span
                  key={`${month.label}-${month.week}`}
                  className="text-[10px] leading-3 text-muted"
                  style={{
                    gridColumn: month.week + 2,
                    gridRow: 1,
                    justifySelf: 'start',
                    overflow: 'visible',
                    whiteSpace: 'nowrap',
                  }}
                >
                  {month.label}
                </span>
              ))}
              {WEEKDAYS.map((day, weekday) => (
                <span
                  key={day}
                  className="text-[10px] leading-3 text-muted"
                  style={{
                    gridColumn: 1,
                    gridRow: weekday + 2,
                    visibility: weekday % 2 === 1 ? 'visible' : 'hidden',
                  }}
                >
                  {day}
                </span>
              ))}
              {grid.cells.map((cell) => (
                <span
                  key={cell.date}
                  title={`${cell.count} ${cell.count === 1 ? 'contribution' : 'contributions'} on ${cell.date}`}
                  className={cx('rounded-sm', LEVEL_CLASSES[cell.level])}
                  style={{
                    gridColumn: cell.week + 2,
                    gridRow: cell.weekday + 2,
                    width: CELL,
                    height: CELL,
                  }}
                />
              ))}
            </div>
          </div>

          <div className="flex items-center justify-end gap-2 text-[10px] text-muted">
            <span>Less</span>
            {LEVEL_CLASSES.map((level, index) => (
              <span
                key={level}
                className={cx('inline-block rounded-sm', level)}
                style={{ width: CELL, height: CELL }}
                title={`Level ${index}`}
              />
            ))}
            <span>More</span>
          </div>
        </>
      ) : null}
    </div>
  )
}
