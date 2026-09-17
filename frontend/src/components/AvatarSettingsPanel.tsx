import { PersonIcon } from '@primer/octicons-react'

import { buttonClasses } from '../lib/ui'
import { LIBRAVATAR_HOME } from './libravatar'
import { Box, BoxBody, BoxHeader } from './primitives/Box'

export function AvatarSettingsPanel({ email }: { email: string }) {
  return (
    <Box>
      <BoxHeader>
        <span className="text-sm font-medium">Profile picture</span>
      </BoxHeader>
      <BoxBody className="space-y-3 text-sm">
        <p>
          Your avatar is derived from the e-mail address on your account{' '}
          <span className="font-mono">{email}</span> using Libravatar. Create or
          update an avatar for that address and it will appear here automatically.
        </p>
        <a
          href={LIBRAVATAR_HOME}
          target="_blank"
          rel="noreferrer noopener"
          className={buttonClasses('default', 'md')}
        >
          <PersonIcon size={16} aria-hidden="true" />
          Open Libravatar
        </a>
      </BoxBody>
    </Box>
  )
}
