import type { ReactNode } from 'react'

import { Box } from './primitives/Box'

export type AuthLayoutProps = {
  title: string
  subtitle?: ReactNode
  children: ReactNode
  footer?: ReactNode
}

export function AuthLayout({
  title,
  subtitle,
  children,
  footer,
}: AuthLayoutProps) {
  return (
    <div className="mx-auto w-full max-w-sm py-8">
      <div className="mb-4 text-center">
        <h1 className="text-2xl font-semibold">{title}</h1>
        {subtitle ? <p className="mt-1 text-sm text-muted">{subtitle}</p> : null}
      </div>
      <Box className="p-4">{children}</Box>
      {footer ? (
        <div className="mt-4 text-center text-sm text-muted">{footer}</div>
      ) : null}
    </div>
  )
}
