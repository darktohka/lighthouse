import { useState } from 'react'

import { analytics as analyticsApi } from '../api/endpoints'
import {
  DiskUsageChart,
  LargestTagsChart,
  PullsOverTimeChart,
  TopRepositoriesChart,
} from '../components/AnalyticsCharts'
import { ChartCard } from '../components/ChartCard'
import { SelectField } from '../components/FormFields'
import { PageHeader } from '../components/PageHeader'
import { StatCard } from '../components/StatCard'
import { Label } from '../components/primitives/Label'
import {
  ErrorState,
  LoadingState,
} from '../components/primitives/StateViews'
import { useAuth } from '../lib/auth-context'
import { formatBytes, formatNumber } from '../lib/format'
import { useAsync } from '../lib/useAsync'

export function AnalyticsPage() {
  const { namespaces } = useAuth()
  const [namespace, setNamespace] = useState('')

  const state = useAsync(
    (signal) =>
      analyticsApi.overview(namespace || undefined, { signal }),
    `analytics:${namespace}`,
  )

  const data = state.data

  return (
    <div className="space-y-4">
      <PageHeader
        title="Analytics"
        description="Storage efficiency and pull traffic across the registry."
      />

      {namespaces.length > 1 ? (
        <div className="w-64">
          <SelectField
            label="Namespace"
            hint="Scope the report to one namespace."
            value={namespace}
            onChange={(event) => setNamespace(event.target.value)}
            options={[
              { value: '', label: 'All namespaces' },
              ...namespaces.map((item) => ({ value: item.name, label: item.name })),
            ]}
          />
        </div>
      ) : null}

      {state.loading && !data ? (
        <LoadingState label="Computing analytics…" />
      ) : null}
      {state.error ? (
        <ErrorState error={state.error} onRetry={state.reload} />
      ) : null}

      {data ? (
        <>
          <section
            aria-label="Storage summary"
            className="grid grid-cols-2 gap-3 lg:grid-cols-4"
          >
            <StatCard label="Total size" value={formatBytes(data.total_size)} />
            <StatCard label="Unique size" value={formatBytes(data.unique_size)} />
            <StatCard
              label="Shared size"
              value={formatBytes(data.shared_size)}
              hint={
                <span className="inline-flex items-center gap-1">
                  <Label variant={data.shared_percentage > 0 ? 'attention' : 'muted'}>
                    {data.shared_percentage.toFixed(1)}%
                  </Label>
                  of total
                </span>
              }
            />
            <StatCard label="Pulls (30d)" value={formatNumber(data.pull_count_30d)} />
          </section>

          <section
            aria-label="Object counts"
            className="grid grid-cols-2 gap-3 lg:grid-cols-6"
          >
            <StatCard label="Blobs" value={formatNumber(data.blob_count)} />
            <StatCard label="Manifests" value={formatNumber(data.manifest_count)} />
            <StatCard
              label="Repositories"
              value={formatNumber(data.repository_count)}
            />
            <StatCard label="Tags" value={formatNumber(data.tag_count)} />
            <StatCard label="Total pulls" value={formatNumber(data.pull_count)} />
            <StatCard
              label="Shared storage"
              value={formatBytes(data.shared_size)}
            />
          </section>

          <div className="grid gap-4 lg:grid-cols-2">
            <ChartCard
              title="Disk usage per repository"
              description="Total vs unique bytes"
              empty={data.disk_usage_by_repository.length === 0}
            >
              <DiskUsageChart data={data.disk_usage_by_repository} />
            </ChartCard>

            <ChartCard
              title="Largest tags"
              description="Top 10 by total size"
              empty={data.largest_tags.length === 0}
            >
              <LargestTagsChart data={data.largest_tags} />
            </ChartCard>

            <ChartCard
              title="Pulls over time"
              description="Last 30 days"
              empty={data.pulls_over_time.length === 0}
            >
              <PullsOverTimeChart data={data.pulls_over_time} />
            </ChartCard>

            <ChartCard
              title="Top repositories by pulls"
              description="All time"
              empty={data.top_repositories.length === 0}
            >
              <TopRepositoriesChart data={data.top_repositories} />
            </ChartCard>
          </div>
        </>
      ) : null}
    </div>
  )
}
