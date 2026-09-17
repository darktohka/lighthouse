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
const STRIDE = CELL + GAP

const LEVEL_CLASSES = [
  'bg-canvas-subtle border border-border',
  'bg-accent/25',
  'bg-accent/45',
  'bg-accent/70',
  'bg-accent',
] as const

type Cell = { date: string; count: number; level: number }

function levelFor(count: number, max: number): number {
  if (count <= 0 || max <= 0) return 0
  const ratio = count / max
  if (ratio >= 0.75) return 4
  if (ratio >= 0.5) return 3
  if (ratio >= 0.25) return 2
  return 1
}

function isLeapYear(year: number): boolean {
  return (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0
}

function buildGrid(year: number, days: readonly HeatmapDay[]) {
  const byDate = new Map(days.map((day) => [day.date, day.count]))
  const startWeekday = new Date(Date.UTC(year, 0, 1)).getUTCDay()
  const dayCount = isLeapYear(year) ? 366 : 365
  const weeks = Math.ceil((startWeekday + dayCount) / 7)
  const max = days.reduce((acc, day) => Math.max(acc, day.count), 0)

  const cells: Cell[] = []
  const monthLabels: Array<{ week: number; label: string }> = []
  for (let index = 0; index < dayCount; index += 1) {
    const date = new Date(Date.UTC(year, 0, 1 + index))
    const iso = date.toISOString().slice(0, 10)
    const count = byDate.get(iso) ?? 0
    cells.push({ date: iso, count, level: levelFor(count, max) })
    if (iso.slice(8, 10) === '01') {
      monthLabels.push({ week: Math.floor((startWeekday + index) / 7), label: MONTHS[date.getUTCMonth()] })
    }
  }
  return { startWeekday, weeks, cells, monthLabels }
}

export type HeatmapProps = {
  year: number
  data: HeatmapData | null
  loading: boolean
  error: Error | null
  onRetry: () => void
  onYearChange: (year: number) => void
  maxYear: number
}

export function Heatmap({
  year,
  data,
  loading,
  error,
  onRetry,
  onYearChange,
  maxYear,
}: HeatmapProps) {
  const grid = data ? buildGrid(data.year, data.days) : null

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-sm font-semibold">
          {data ? `${formatNumber(data.total)} contributions in ${data.year}` : 'Contributions'}
        </h3>
        <div className="flex items-center gap-1">
          <Button
            size="sm"
            disabled={year <= 2000}
            aria-label="Previous year"
            onClick={() => onYearChange(year - 1)}
            leadingIcon={<ChevronLeftIcon size={14} aria-hidden="true" />}
          >
            {year - 1}
          </Button>
          <Button
            size="sm"
            disabled={year >= maxYear}
            aria-label="Next year"
            onClick={() => onYearChange(year + 1)}
            trailingIcon={<ChevronRightIcon size={14} aria-hidden="true" />}
          >
            {year + 1}
          </Button>
        </div>
      </div>

      {loading && !data ? <LoadingState label="Loading contributions…" /> : null}
      {error ? <ErrorState error={error} onRetry={onRetry} /> : null}

      {grid && data ? (
        <>
          <div className="overflow-x-auto pb-1 scrollbar-thin">
            <div
              className="inline-block"
              style={{ minWidth: grid.weeks * STRIDE + 30 }}
            >
              <div className="relative mb-1 h-3" style={{ width: grid.weeks * STRIDE }}>
                {grid.monthLabels.map((month) => (
                  <span
                    key={`${month.label}-${month.week}`}
                    className="absolute top-0 text-[10px] leading-3 text-muted"
                    style={{ left: month.week * STRIDE }}
                  >
                    {month.label}
                  </span>
                ))}
              </div>
              <div className="flex gap-1">
                <div
                  className="flex flex-col text-[10px] text-muted"
                  style={{ gap: GAP, paddingTop: 0 }}
                >
                  {WEEKDAYS.map((day, index) => (
                    <span
                      key={day}
                      className="leading-3"
                      style={{ height: CELL, visibility: index % 2 === 1 ? 'visible' : 'hidden' }}
                    >
                      {day.slice(0, 3)}
                    </span>
                  ))}
                </div>
                <div
                  role="img"
                  aria-label={`Contribution heatmap for ${data.year}`}
                  className="grid"
                  style={{
                    gap: GAP,
                    gridAutoFlow: 'column',
                    gridTemplateColumns: `repeat(${grid.weeks}, ${CELL}px)`,
                    gridTemplateRows: `repeat(7, ${CELL}px)`,
                  }}
                >
                  {Array.from({ length: grid.startWeekday }, (_, index) => (
                    <span key={`pad-${index}`} style={{ width: CELL, height: CELL }} />
                  ))}
                  {grid.cells.map((cell) => (
                    <span
                      key={cell.date}
                      title={`${cell.count} ${cell.count === 1 ? 'contribution' : 'contributions'} on ${cell.date}`}
                      className={cx('rounded-sm', LEVEL_CLASSES[cell.level])}
                      style={{ width: CELL, height: CELL }}
                    />
                  ))}
                </div>
              </div>
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
