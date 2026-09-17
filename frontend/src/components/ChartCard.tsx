import type { ReactNode } from 'react'

import { Box, BoxHeader } from './primitives/Box'
import { EmptyState } from './primitives/StateViews'

export type ChartCardProps = {
  title: string
  description?: string
  empty: boolean
  children: ReactNode
}

export function ChartCard({ title, description, empty, children }: ChartCardProps) {
  return (
    <Box>
      <BoxHeader>
        <span className="text-sm font-medium">{title}</span>
        {description ? (
          <span className="text-xs text-muted">{description}</span>
        ) : null}
      </BoxHeader>
      <div className="p-3">
        {empty ? (
          <EmptyState
            title="No data yet"
            description="Pull statistics and storage usage will appear once images are pushed and pulled."
          />
        ) : (
          <div className="h-64 w-full">{children}</div>
        )}
      </div>
    </Box>
  )
}
