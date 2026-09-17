import type { ButtonHTMLAttributes, ReactNode } from 'react'
import { Link } from 'react-router-dom'

import {
  buttonClasses,
  type ButtonSize,
  type ButtonVariant,
} from '../../lib/ui'

type SharedButtonProps = {
  variant?: ButtonVariant
  size?: ButtonSize
  leadingIcon?: ReactNode
  trailingIcon?: ReactNode
}

export type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> &
  SharedButtonProps

export function Button({
  variant = 'default',
  size = 'md',
  leadingIcon,
  trailingIcon,
  className,
  children,
  type = 'button',
  ...rest
}: ButtonProps) {
  return (
    <button
      type={type}
      className={buttonClasses(variant, size, className)}
      {...rest}
    >
      {leadingIcon}
      {children}
      {trailingIcon}
    </button>
  )
}

export type LinkButtonProps = {
  to: string
  variant?: ButtonVariant
  size?: ButtonSize
  leadingIcon?: ReactNode
  trailingIcon?: ReactNode
  className?: string
  children?: ReactNode
  'aria-label'?: string
  title?: string
}

/** Internal navigation styled as a button. */
export function LinkButton({
  to,
  variant = 'default',
  size = 'md',
  leadingIcon,
  trailingIcon,
  className,
  children,
  ...rest
}: LinkButtonProps) {
  return (
    <Link to={to} className={buttonClasses(variant, size, className)} {...rest}>
      {leadingIcon}
      {children}
      {trailingIcon}
    </Link>
  )
}

export type AnchorButtonProps = {
  href: string
  variant?: ButtonVariant
  size?: ButtonSize
  leadingIcon?: ReactNode
  trailingIcon?: ReactNode
  className?: string
  children?: ReactNode
  download?: boolean | string
  'aria-label'?: string
  title?: string
}

/** External / download link styled as a button. */
export function AnchorButton({
  href,
  variant = 'default',
  size = 'md',
  leadingIcon,
  trailingIcon,
  className,
  children,
  ...rest
}: AnchorButtonProps) {
  return (
    <a href={href} className={buttonClasses(variant, size, className)} {...rest}>
      {leadingIcon}
      {children}
      {trailingIcon}
    </a>
  )
}
