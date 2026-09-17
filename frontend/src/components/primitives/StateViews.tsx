import { PackageIcon, SyncIcon } from '@primer/octicons-react'
import type { ReactNode } from 'react'

import { isApiError } from '../../api/client'
import { cx } from '../../lib/cx'
import { Button } from './Button'
import { Flash } from './Flash'

export function Spinner({ className }: { className?: string }) {
  return (
    <SyncIcon
      size={16}
      aria-hidden="true"
      className={cx('animate-spin', className)}
    />
  )
}

export type LoadingStateProps = {
  label?: string
  className?: string
}

export function LoadingState({
  label = 'Loading…',
  className,
}: LoadingStateProps) {
  return (
    <div
      role="status"
      className={cx(
        'flex items-center justify-center gap-2 py-12 text-sm text-muted',
        className,
      )}
    >
      <Spinner />
      <span>{label}</span>
    </div>
  )
}

export type EmptyStateProps = {
  title: ReactNode
  description?: ReactNode
  icon?: ReactNode
  action?: ReactNode
  className?: string
}

export function EmptyState({
  title,
  description,
  icon,
  action,
  className,
}: EmptyStateProps) {
  return (
    <div
      className={cx(
        'flex flex-col items-center gap-2 px-4 py-12 text-center',
        className,
      )}
    >
      <span className="text-muted">
        {icon ?? <PackageIcon size={24} aria-hidden="true" />}
      </span>
      <h2 className="text-base font-semibold">{title}</h2>
      {description ? (
        <p className="max-w-md text-sm text-muted">{description}</p>
      ) : null}
      {action ? <div className="mt-2">{action}</div> : null}
    </div>
  )
}

export type ErrorStateProps = {
  error: Error
  onRetry?: () => void
  className?: string
}

function errorTitle(error: Error): string {
  if (isApiError(error)) {
    if (error.status === 404) return 'Not found'
    if (error.status === 403) return 'Access denied'
    if (error.status === 401) return 'Authentication required'
    if (error.status === 429) return 'Too many requests'
  }
  return 'Something went wrong'
}

export function ErrorState({ error, onRetry, className }: ErrorStateProps) {
  return (
    <Flash variant="danger" title={errorTitle(error)} className={className}>
      <div className="space-y-2">
        <p>{error.message}</p>
        {onRetry ? (
          <Button size="sm" onClick={onRetry}>
            Try again
          </Button>
        ) : null}
      </div>
    </Flash>
  )
}
