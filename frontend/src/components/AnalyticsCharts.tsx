import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts'

import type {
  DiskUsageEntry,
  PullsOverTime,
  TagSizeEntry,
  TopRepository,
} from '../api/schemas'
import { formatBytes, formatNumber } from '../lib/format'
import { TOOLTIP_STYLE, useChartColors } from './chartTheme'

function shortRepository(path: string): string {
  const segments = path.split('/').filter(Boolean)
  return segments.length > 2 ? `…/${segments.slice(-2).join('/')}` : path
}

export function DiskUsageChart({ data }: { data: DiskUsageEntry[] }) {
  const colors = useChartColors()
  const rows = data.slice(0, 12)
  return (
    <ResponsiveContainer width="100%" height="100%">
      <BarChart
        data={rows}
        layout="vertical"
        margin={{ top: 4, right: 16, bottom: 4, left: 8 }}
      >
        <CartesianGrid stroke={colors.grid} horizontal={false} />
        <XAxis
          type="number"
          tickFormatter={(value) => formatBytes(Number(value))}
          tick={{ fill: colors.muted, fontSize: 11 }}
          axisLine={{ stroke: colors.border }}
          tickLine={{ stroke: colors.border }}
        />
        <YAxis
          type="category"
          dataKey="repository"
          width={150}
          tickFormatter={shortRepository}
          tick={{ fill: colors.muted, fontSize: 11 }}
          axisLine={{ stroke: colors.border }}
          tickLine={{ stroke: colors.border }}
        />
        <Tooltip
          contentStyle={TOOLTIP_STYLE}
          formatter={(value, name) => [formatBytes(Number(value)), name]}
          cursor={{ fill: 'var(--lh-neutral-subtle)' }}
        />
        <Bar dataKey="size" name="Total" fill={colors.accent} radius={3} />
        <Bar dataKey="unique_size" name="Unique" fill={colors.success} radius={3} />
      </BarChart>
    </ResponsiveContainer>
  )
}

export function LargestTagsChart({ data }: { data: TagSizeEntry[] }) {
  const colors = useChartColors()
  const rows = data.slice(0, 10).map((entry) => ({
    label: `${entry.namespace}/${entry.repository}:${entry.tag}`,
    total_size: entry.total_size,
  }))
  return (
    <ResponsiveContainer width="100%" height="100%">
      <BarChart data={rows} margin={{ top: 4, right: 16, bottom: 48, left: 8 }}>
        <CartesianGrid stroke={colors.grid} vertical={false} />
        <XAxis
          dataKey="label"
          tickFormatter={shortRepository}
          tick={{ fill: colors.muted, fontSize: 11 }}
          angle={-30}
          textAnchor="end"
          interval={0}
          height={60}
          axisLine={{ stroke: colors.border }}
          tickLine={{ stroke: colors.border }}
        />
        <YAxis
          tickFormatter={(value) => formatBytes(Number(value))}
          tick={{ fill: colors.muted, fontSize: 11 }}
          axisLine={{ stroke: colors.border }}
          tickLine={{ stroke: colors.border }}
        />
        <Tooltip
          contentStyle={TOOLTIP_STYLE}
          formatter={(value) => [formatBytes(Number(value)), 'Total size']}
          cursor={{ fill: 'var(--lh-neutral-subtle)' }}
        />
        <Bar dataKey="total_size" fill={colors.accent} radius={3} />
      </BarChart>
    </ResponsiveContainer>
  )
}

export function PullsOverTimeChart({ data }: { data: PullsOverTime[] }) {
  const colors = useChartColors()
  return (
    <ResponsiveContainer width="100%" height="100%">
      <AreaChart data={data} margin={{ top: 4, right: 16, bottom: 4, left: 8 }}>
        <defs>
          <linearGradient id="lh-pulls" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor={colors.accent} stopOpacity={0.4} />
            <stop offset="100%" stopColor={colors.accent} stopOpacity={0.05} />
          </linearGradient>
        </defs>
        <CartesianGrid stroke={colors.grid} vertical={false} />
        <XAxis
          dataKey="date"
          tick={{ fill: colors.muted, fontSize: 11 }}
          axisLine={{ stroke: colors.border }}
          tickLine={{ stroke: colors.border }}
        />
        <YAxis
          allowDecimals={false}
          tickFormatter={(value) => formatNumber(Number(value))}
          tick={{ fill: colors.muted, fontSize: 11 }}
          axisLine={{ stroke: colors.border }}
          tickLine={{ stroke: colors.border }}
        />
        <Tooltip
          contentStyle={TOOLTIP_STYLE}
          formatter={(value) => [formatNumber(Number(value)), 'Pulls']}
        />
        <Area
          type="monotone"
          dataKey="pulls"
          stroke={colors.accent}
          strokeWidth={2}
          fill="url(#lh-pulls)"
        />
      </AreaChart>
    </ResponsiveContainer>
  )
}

export function TopRepositoriesChart({ data }: { data: TopRepository[] }) {
  const colors = useChartColors()
  const rows = data.slice(0, 10)
  return (
    <ResponsiveContainer width="100%" height="100%">
      <BarChart data={rows} margin={{ top: 4, right: 16, bottom: 48, left: 8 }}>
        <CartesianGrid stroke={colors.grid} vertical={false} />
        <XAxis
          dataKey="repository"
          tickFormatter={shortRepository}
          tick={{ fill: colors.muted, fontSize: 11 }}
          angle={-30}
          textAnchor="end"
          interval={0}
          height={60}
          axisLine={{ stroke: colors.border }}
          tickLine={{ stroke: colors.border }}
        />
        <YAxis
          allowDecimals={false}
          tickFormatter={(value) => formatNumber(Number(value))}
          tick={{ fill: colors.muted, fontSize: 11 }}
          axisLine={{ stroke: colors.border }}
          tickLine={{ stroke: colors.border }}
        />
        <Tooltip
          contentStyle={TOOLTIP_STYLE}
          formatter={(value) => [formatNumber(Number(value)), 'Pulls']}
          cursor={{ fill: 'var(--lh-neutral-subtle)' }}
        />
        <Bar dataKey="pulls" fill={colors.success} radius={3} />
      </BarChart>
    </ResponsiveContainer>
  )
}
