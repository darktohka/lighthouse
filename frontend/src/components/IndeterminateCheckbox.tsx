import { useEffect, useRef, type InputHTMLAttributes } from 'react'

import { cx } from '../lib/cx'

export type IndeterminateCheckboxProps = Omit<
  InputHTMLAttributes<HTMLInputElement>,
  'type'
> & {
  /** Renders the dash state; the checkbox's `checked` value is unaffected. */
  indeterminate?: boolean
}

/** Checkbox that also supports the native mixed (`indeterminate`) state. */
export function IndeterminateCheckbox({
  indeterminate = false,
  className,
  ...rest
}: IndeterminateCheckboxProps) {
  const ref = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (ref.current) ref.current.indeterminate = indeterminate
  }, [indeterminate])

  return (
    <input
      ref={ref}
      type="checkbox"
      className={cx('h-3.5 w-3.5 cursor-pointer align-middle', className)}
      {...rest}
    />
  )
}
