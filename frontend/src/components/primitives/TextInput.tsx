import { useId, type InputHTMLAttributes, type ReactNode } from 'react'

import { cx } from '../../lib/cx'
import { inputClasses } from '../../lib/ui'

export type TextInputProps = Omit<
  InputHTMLAttributes<HTMLInputElement>,
  'id'
> & {
  label: string
  hint?: ReactNode
  error?: string | null
  id?: string
}

/** Labelled text input with hint and inline error wiring. */
export function TextInput({
  label,
  hint,
  error,
  id,
  className,
  required,
  ...rest
}: TextInputProps) {
  const generatedId = useId()
  const inputId = id ?? generatedId
  const hintId = `${inputId}-hint`
  const errorId = `${inputId}-error`
  const describedBy =
    [hint ? hintId : null, error ? errorId : null].filter(Boolean).join(' ') ||
    undefined

  return (
    <div className="space-y-1">
      <label htmlFor={inputId} className="block text-sm font-medium">
        {label}
        {required ? (
          <span className="ml-0.5 text-danger" aria-hidden="true">
            *
          </span>
        ) : null}
      </label>
      {hint ? (
        <p id={hintId} className="text-xs text-muted">
          {hint}
        </p>
      ) : null}
      <input
        id={inputId}
        required={required}
        aria-invalid={error ? true : undefined}
        aria-describedby={describedBy}
        className={inputClasses(
          cx(error ? 'border-danger focus:border-danger' : undefined, className),
        )}
        {...rest}
      />
      {error ? (
        <p id={errorId} className="text-xs text-danger">
          {error}
        </p>
      ) : null}
    </div>
  )
}
