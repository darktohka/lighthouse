import { useId, type ChangeEvent, type ReactNode } from 'react'

import { cx } from '../lib/cx'
import { inputClasses } from '../lib/ui'

type FieldShellProps = {
  label: string
  hint?: ReactNode
  error?: string | null
  controlId: string
  required?: boolean
  children: ReactNode
}

function FieldShell({
  label,
  hint,
  error,
  controlId,
  required,
  children,
}: FieldShellProps) {
  const hintId = `${controlId}-hint`
  const errorId = `${controlId}-error`
  return (
    <div className="space-y-1">
      <label htmlFor={controlId} className="block text-sm font-medium">
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
      {children}
      {error ? (
        <p id={errorId} className="text-xs text-danger">
          {error}
        </p>
      ) : null}
    </div>
  )
}

export type TextAreaProps = {
  label: string
  hint?: ReactNode
  error?: string | null
  id?: string
  required?: boolean
  rows?: number
  value: string
  onChange: (event: ChangeEvent<HTMLTextAreaElement>) => void
  placeholder?: string
  className?: string
}

export function TextArea({
  label,
  hint,
  error,
  id,
  required,
  rows = 3,
  className,
  ...rest
}: TextAreaProps) {
  const generatedId = useId()
  const controlId = id ?? generatedId
  return (
    <FieldShell
      label={label}
      hint={hint}
      error={error}
      controlId={controlId}
      required={required}
    >
      <textarea
        id={controlId}
        rows={rows}
        required={required}
        aria-invalid={error ? true : undefined}
        aria-describedby={
          [hint ? `${controlId}-hint` : null, error ? `${controlId}-error` : null]
            .filter(Boolean)
            .join(' ') || undefined
        }
        className={inputClasses(
          cx('resize-y', error ? 'border-danger' : undefined, className),
        )}
        {...rest}
      />
    </FieldShell>
  )
}

export type SelectOption = { value: string; label: string }

export type SelectFieldProps = {
  label: string
  hint?: ReactNode
  error?: string | null
  id?: string
  required?: boolean
  value: string
  onChange: (event: ChangeEvent<HTMLSelectElement>) => void
  options: readonly SelectOption[]
  className?: string
}

export function SelectField({
  label,
  hint,
  error,
  id,
  required,
  options,
  className,
  ...rest
}: SelectFieldProps) {
  const generatedId = useId()
  const controlId = id ?? generatedId
  return (
    <FieldShell
      label={label}
      hint={hint}
      error={error}
      controlId={controlId}
      required={required}
    >
      <select
        id={controlId}
        required={required}
        aria-invalid={error ? true : undefined}
        className={inputClasses(
          cx(error ? 'border-danger' : undefined, className),
        )}
        {...rest}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    </FieldShell>
  )
}

export type SwitchFieldProps = {
  label: string
  hint?: ReactNode
  checked: boolean
  onCheckedChange: (checked: boolean) => void
  disabled?: boolean
  className?: string
}

export function SwitchField({
  label,
  hint,
  checked,
  onCheckedChange,
  disabled,
  className,
}: SwitchFieldProps) {
  return (
    <label
      className={cx(
        'flex cursor-pointer items-start gap-2',
        disabled ? 'cursor-not-allowed opacity-60' : undefined,
        className,
      )}
    >
      <input
        type="checkbox"
        role="switch"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onCheckedChange(event.target.checked)}
        className="peer sr-only"
      />
      <span
        aria-hidden="true"
        className={cx(
          'mt-0.5 inline-flex h-5 w-9 shrink-0 items-center rounded-full border transition-colors peer-focus-visible:outline peer-focus-visible:outline-2 peer-focus-visible:outline-accent',
          checked
            ? 'border-accent-emphasis bg-accent-emphasis'
            : 'border-border bg-canvas-subtle',
        )}
      >
        <span
          className={cx(
            'inline-block h-3.5 w-3.5 rounded-full bg-canvas-default transition-transform',
            checked ? 'translate-x-[18px]' : 'translate-x-[3px]',
          )}
        />
      </span>
      <span className="text-sm">
        <span className="font-medium">{label}</span>
        {hint ? <span className="block text-xs text-muted">{hint}</span> : null}
      </span>
    </label>
  )
}
