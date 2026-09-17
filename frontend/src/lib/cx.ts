export type ClassValue = string | false | null | undefined

/** Joins conditional class names. */
export function cx(...values: ClassValue[]): string {
  return values.filter((value): value is string => Boolean(value)).join(' ')
}
