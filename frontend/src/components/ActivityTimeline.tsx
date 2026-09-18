import { BroadcastIcon } from '@primer/octicons-react'

import type { ActivityEntry } from '../api/schemas'
import { formatDateTime, formatRelativeTime } from '../lib/format'
import { LibravatarAvatar } from './LibravatarAvatar'
import { Label } from './primitives/Label'
import { EmptyState } from './primitives/StateViews'
import { Timeline, TimelineItem } from './primitives/Timeline'

export type ActivityTimelineProps = {
  entries: readonly ActivityEntry[]
  emptyDescription?: string
}

export function ActivityTimeline({
  entries,
  emptyDescription,
}: ActivityTimelineProps) {
  if (entries.length === 0) {
    return (
      <EmptyState
        title="No activity yet"
        description={
          emptyDescription ??
          'Pushes, tag deletions and permission changes will appear here.'
        }
        icon={<BroadcastIcon size={24} aria-hidden="true" />}
      />
    )
  }

  return (
    <Timeline>
      {entries.map((entry) => (
        <TimelineItem
          key={entry.id}
          avatar={
            entry.actor ? (
              <LibravatarAvatar
                src={entry.actor.avatar_url}
                hash={entry.actor.avatar_hash}
                name={entry.actor.username}
                size={32}
              />
            ) : undefined
          }
          icon={<BroadcastIcon size={14} aria-hidden="true" />}
        >
          <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
            <p className="text-sm">{entry.summary}</p>
            <time
              className="text-xs text-muted"
              dateTime={entry.created_at}
              title={formatDateTime(entry.created_at)}
            >
              {formatRelativeTime(entry.created_at)}
            </time>
          </div>
          <div className="mt-1 flex flex-wrap items-center gap-1.5">
            <Label variant="muted" className="font-mono">
              {entry.kind}
            </Label>
            {entry.repository ? (
              <span className="font-mono text-xs text-muted">
                {entry.repository}
              </span>
            ) : null}
            {entry.namespace ? (
              <span className="text-xs text-muted">· {entry.namespace}</span>
            ) : null}
          </div>
        </TimelineItem>
      ))}
    </Timeline>
  )
}
