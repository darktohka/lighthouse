import { SearchIcon } from '@primer/octicons-react'
import { useState, type FormEvent } from 'react'
import { useNavigate } from 'react-router-dom'

import { inputClasses } from '../lib/ui'

export function SearchBox({ className }: { className?: string }) {
  const [value, setValue] = useState('')
  const navigate = useNavigate()

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const query = value.trim()
    navigate(query.length > 0 ? `/explore?q=${encodeURIComponent(query)}` : '/explore')
  }

  return (
    <form role="search" onSubmit={onSubmit} className={className}>
      <label htmlFor="global-search" className="sr-only">
        Search namespaces and repositories
      </label>
      <div className="relative">
        <SearchIcon
          size={16}
          aria-hidden="true"
          className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-muted"
        />
        <input
          id="global-search"
          type="search"
          value={value}
          onChange={(event) => setValue(event.target.value)}
          placeholder="Search namespaces and repositories"
          className={inputClasses('h-8 pl-8')}
        />
      </div>
    </form>
  )
}
