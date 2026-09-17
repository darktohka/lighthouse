/**
 * Form validation built on Valibot. `validateForm` mirrors a form schema into
 * per-field error messages via `v.flatten`, which is what powers the inline
 * validation on the auth pages.
 */
import * as v from 'valibot'

export type FieldErrors = Record<string, string>

export type ValidationResult<TOutput> =
  | { ok: true; value: TOutput }
  | { ok: false; errors: FieldErrors; formError: string | null }

export function validateForm<TInput, TOutput>(
  schema: v.GenericSchema<TInput, TOutput>,
  input: TInput,
): ValidationResult<TOutput> {
  const result = v.safeParse(schema, input)
  if (result.success) return { ok: true, value: result.output }

  const flat = v.flatten(result.issues)
  const errors: FieldErrors = {}
  if (flat.nested) {
    for (const [field, messages] of Object.entries(flat.nested)) {
      if (messages && messages.length > 0) {
        errors[field] = messages[0]
      }
    }
  }
  const formError = flat.root?.[0] ?? null
  return { ok: false, errors, formError }
}
