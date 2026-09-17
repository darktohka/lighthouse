import type { ReactNode } from 'react'

import { Box } from './primitives/Box'

export type StatCardProps = {
  label: string
  value: ReactNode
  hint?: ReactNode
}

export function StatCard({ label, value, hint }: StatCardProps) {
  return (
    <Box className="p-3">
      <p className="text-xs text-muted">{label}</p>
      <p className="mt-1 text-xl font-semibold">{value}</p>
      {hint ? <p className="mt-0.5 text-xs text-muted">{hint}</p> : null}
    </Box>
  )
}
