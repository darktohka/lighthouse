import { Link } from 'react-router-dom'

import { EmptyState } from '../components/primitives/StateViews'

export function NotFoundPage() {
  return (
    <EmptyState
      title="Page not found"
      description="The page you were looking for does not exist."
      action={
        <Link to="/" className="text-sm text-accent hover:underline">
          Back to Explore
        </Link>
      }
    />
  )
}
