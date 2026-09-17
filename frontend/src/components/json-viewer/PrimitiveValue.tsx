import { Highlight } from './Highlight'
import { primitiveText } from './json-model'

export function PrimitiveValue({
  value,
  query,
}: {
  value: unknown
  query: string
}) {
  if (typeof value === 'string') {
    return (
      <span className="text-success">
        &quot;
        <Highlight text={value} query={query} />
        &quot;
      </span>
    )
  }
  if (typeof value === 'number') {
    return (
      <span className="text-attention">
        <Highlight text={String(value)} query={query} />
      </span>
    )
  }
  if (typeof value === 'boolean') {
    return (
      <span className="text-danger">
        <Highlight text={String(value)} query={query} />
      </span>
    )
  }
  return (
    <span className="text-muted">
      <Highlight text={primitiveText(value)} query={query} />
    </span>
  )
}
