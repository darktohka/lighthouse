import {
  KeyIcon,
  PlusIcon,
  SearchIcon,
  TagIcon,
  TrashIcon,
} from '@primer/octicons-react'
import { useEffect, useId, useMemo, useRef, useState, type FormEvent } from 'react'

import { isApiError } from '../api/client'
import {
  appPasswordCategories as categoriesApi,
  appPasswords,
} from '../api/endpoints'
import {
  createAppPasswordCategorySchema,
  type AppPassword,
  type AppPasswordCategory,
} from '../api/schemas'
import { validateForm } from '../lib/forms'
import { compactInputClasses } from '../lib/ui'
import { useAsync } from '../lib/useAsync'
import { AppPasswordCard } from './AppPasswordCard'
import { CreateAppPasswordDialog } from './CreateAppPasswordDialog'
import { Box, BoxHeader } from './primitives/Box'
import { Button } from './primitives/Button'
import { ConfirmAction } from './ConfirmAction'
import { Flash } from './primitives/Flash'
import { Label } from './primitives/Label'
import { EmptyState, ErrorState, LoadingState } from './primitives/StateViews'

const UNCATEGORIZED_KEY = '__uncategorized__'

type Section = {
  key: string
  category: AppPasswordCategory | null
  /** App passwords that survive the active search. */
  passwords: AppPassword[]
  /** Total before filtering, so a filtered section can say "1 of 3". */
  total: number
}

/** Lowercased haystack of everything the search should look at. */
function searchIndex(
  appPassword: AppPassword,
  category: AppPasswordCategory | null,
): string {
  return [
    appPassword.name,
    appPassword.token_prefix,
    appPassword.token_suffix,
    category?.name ?? 'uncategorized',
  ]
    .join(' ')
    .toLowerCase()
}

export function AppPasswordsPanel() {
  const passwordsState = useAsync(
    (signal) => appPasswords.list({ signal }),
    'app-passwords',
  )
  const categoriesState = useAsync(
    (signal) => categoriesApi.list({ signal }),
    'app-password-categories',
  )

  const [query, setQuery] = useState('')
  const [createOpen, setCreateOpen] = useState(false)
  const [createCategoryFor, setCreateCategoryFor] = useState<number | null>(null)
  /** Token shown once, inside the app password it belongs to. */
  const [issued, setIssued] = useState<{
    appPasswordId: number
    token: string
    label: string
  } | null>(null)
  /** Only one app password is expanded at a time; null means all collapsed. */
  const [expandedId, setExpandedId] = useState<number | null>(null)
  const scrollTargetRef = useRef<number | null>(null)

  const [newCategoryName, setNewCategoryName] = useState('')
  const [creatingCategory, setCreatingCategory] = useState(false)
  const [categoryError, setCategoryError] = useState<string | null>(null)
  const [removingCategory, setRemovingCategory] = useState<number | null>(null)

  const searchId = useId()
  const newCategoryId = useId()

  const passwords = useMemo(() => passwordsState.data ?? [], [passwordsState.data])
  const categories = useMemo(
    () => categoriesState.data ?? [],
    [categoriesState.data],
  )

  // After a creation the list is re-fetched; scroll once the new row exists.
  useEffect(() => {
    const target = scrollTargetRef.current
    if (target === null) return
    const row = document.querySelector<HTMLElement>(
      `[data-app-password="${target}"]`,
    )
    if (!row) return
    scrollTargetRef.current = null
    row.scrollIntoView({ block: 'center', behavior: 'smooth' })
  }, [passwords, expandedId])

  const reloadAll = () => {
    passwordsState.reload()
    categoriesState.reload()
  }

  /** Reveal a freshly minted token in the row it belongs to. */
  const showToken = (appPasswordId: number, token: string, label: string) => {
    setIssued({ appPasswordId, token, label })
    setExpandedId(appPasswordId)
  }

  const createCategory = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    setCategoryError(null)
    const validation = validateForm(createAppPasswordCategorySchema, {
      name: newCategoryName,
    })
    if (!validation.ok) {
      setCategoryError(validation.errors.name ?? 'Give the category a name.')
      return
    }
    const name = validation.value.name
    setCreatingCategory(true)
    void categoriesApi.create({ name }).then(
      () => {
        setCreatingCategory(false)
        setNewCategoryName('')
        categoriesState.reload()
      },
      (error: unknown) => {
        setCreatingCategory(false)
        setCategoryError(
          isApiError(error)
            ? error.status === 409
              ? `A category called “${name}” already exists.`
              : error.message
            : 'The category could not be created.',
        )
      },
    )
  }

  const removeCategory = (category: AppPasswordCategory) => {
    setCategoryError(null)
    setRemovingCategory(category.id)
    void categoriesApi.remove(category.id).then(
      () => {
        setRemovingCategory(null)
        reloadAll()
      },
      (error: unknown) => {
        setRemovingCategory(null)
        setCategoryError(
          isApiError(error)
            ? error.status === 409
              ? `“${category.name}” is not empty — move its app passwords first.`
              : error.message
            : 'The category could not be deleted.',
        )
      },
    )
  }

  const normalizedQuery = query.trim().toLowerCase()

  const sections = useMemo<Section[]>(() => {
    const byCategory = new Map<number, AppPassword[]>()
    const uncategorized: AppPassword[] = []
    const knownCategoryIds = new Set(categories.map((entry) => entry.id))
    for (const appPassword of passwords) {
      // A password pointing at a deleted category falls back to "Uncategorized".
      if (
        appPassword.category_id === null ||
        !knownCategoryIds.has(appPassword.category_id)
      ) {
        uncategorized.push(appPassword)
      } else {
        const bucket = byCategory.get(appPassword.category_id)
        if (bucket) bucket.push(appPassword)
        else byCategory.set(appPassword.category_id, [appPassword])
      }
    }

    const matches = (
      appPassword: AppPassword,
      category: AppPasswordCategory | null,
    ) => {
      if (normalizedQuery.length === 0) return true
      return searchIndex(appPassword, category).includes(normalizedQuery)
    }

    // Alphabetical by name (case-insensitive) within every section.
    const byName = (left: AppPassword, right: AppPassword) =>
      left.name.localeCompare(right.name, undefined, { sensitivity: 'base' })

    const result: Section[] = categories.map((category) => {
      const bucket = byCategory.get(category.id) ?? []
      return {
        key: `category-${category.id}`,
        category,
        passwords: bucket
          .filter((appPassword) => matches(appPassword, category))
          .sort(byName),
        total: bucket.length,
      }
    })

    const uncategorizedMatches = uncategorized
      .filter((appPassword) => matches(appPassword, null))
      .sort(byName)
    if (uncategorizedMatches.length > 0) {
      result.push({
        key: UNCATEGORIZED_KEY,
        category: null,
        passwords: uncategorizedMatches,
        total: uncategorized.length,
      })
    }

    return result
  }, [passwords, categories, normalizedQuery])

  const matchesAny = sections.some((section) => section.passwords.length > 0)
  const totalPasswords = passwords.length

  const loading =
    (passwordsState.loading && !passwordsState.data) ||
    (categoriesState.loading && !categoriesState.data)
  const error = passwordsState.error ?? categoriesState.error

  return (
    <div className="space-y-3">
      <Box>
        <BoxHeader className="gap-2">
          <span className="flex items-center gap-2 text-sm font-medium">
            <KeyIcon size={16} aria-hidden="true" />
            App passwords
          </span>
          <div className="flex flex-wrap items-center gap-2">
            <div className="relative w-56 sm:w-64">
              <label htmlFor={searchId} className="sr-only">
                Search app passwords
              </label>
              <SearchIcon
                size={14}
                aria-hidden="true"
                className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-muted"
              />
              <input
                id={searchId}
                type="search"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder="Search app passwords…"
                className={compactInputClasses('h-7 pl-7')}
              />
            </div>
            <Button
              size="sm"
              variant="success"
              leadingIcon={<PlusIcon size={14} aria-hidden="true" />}
              onClick={() => setCreateOpen(true)}
            >
              New app password
            </Button>
          </div>
        </BoxHeader>

        {loading ? <LoadingState label="Loading app passwords…" /> : null}
        {error ? <ErrorState error={error} onRetry={reloadAll} className="m-3" /> : null}

        {!loading && !error ? (
          <div className="divide-y divide-border">
            {sections.map((section) => (
              <CategorySection
                key={section.key}
                section={section}
                categories={categories}
                query={normalizedQuery}
                removingCategory={removingCategory}
                onRemoveCategory={removeCategory}
                onCreatePasswordHere={
                  section.category
                    ? () => setCreateCategoryFor(section.category?.id ?? null)
                    : null
                }
                expandedId={expandedId}
                onTogglePassword={(appPasswordId) =>
                  setExpandedId((current) =>
                    current === appPasswordId ? null : appPasswordId,
                  )
                }
                issued={issued}
                onTokenDismiss={() => setIssued(null)}
                onChanged={reloadAll}
                onToken={showToken}
              />
            ))}

            {totalPasswords === 0 && categories.length === 0 ? (
              <EmptyState
                title="No app passwords yet"
                description="Create one to run docker login without your account password or two-factor code."
                icon={<KeyIcon size={24} aria-hidden="true" />}
              />
            ) : null}

            {totalPasswords > 0 && !matchesAny ? (
              <p className="px-4 py-8 text-center text-sm text-muted">
                No app passwords match “{query.trim()}”
              </p>
            ) : null}

            <CategoryCreator
              id={newCategoryId}
              value={newCategoryName}
              onChange={setNewCategoryName}
              onSubmit={createCategory}
              pending={creatingCategory}
            />
          </div>
        ) : null}

        {categoryError ? (
          <div className="border-t border-border p-3">
            <Flash variant="danger" onDismiss={() => setCategoryError(null)}>
              {categoryError}
            </Flash>
          </div>
        ) : null}
      </Box>

      {createOpen || createCategoryFor !== null ? (
        <CreateAppPasswordDialog
          key={createCategoryFor === null ? 'new' : `category-${createCategoryFor}`}
          categories={categories}
          initialCategoryId={createCategoryFor}
          onClose={() => {
            setCreateOpen(false)
            setCreateCategoryFor(null)
          }}
          onCreated={(result) => {
            // Collapse every other row, open the new one, then scroll to it
            // once the refreshed list contains it.
            setExpandedId(result.appPasswordId)
            scrollTargetRef.current = result.appPasswordId
            showToken(result.appPasswordId, result.token, result.label)
            reloadAll()
          }}
        />
      ) : null}
    </div>
  )
}

type CategorySectionProps = {
  section: Section
  categories: AppPasswordCategory[]
  query: string
  removingCategory: number | null
  onRemoveCategory: (category: AppPasswordCategory) => void
  onCreatePasswordHere: (() => void) | null
  expandedId: number | null
  onTogglePassword: (appPasswordId: number) => void
  issued: { appPasswordId: number; token: string; label: string } | null
  onTokenDismiss: () => void
  onChanged: () => void
  onToken: (appPasswordId: number, token: string, label: string) => void
}

function CategorySection({
  section,
  categories,
  query,
  removingCategory,
  onRemoveCategory,
  onCreatePasswordHere,
  expandedId,
  onTogglePassword,
  issued,
  onTokenDismiss,
  onChanged,
  onToken,
}: CategorySectionProps) {
  const { category, passwords } = section
  const filtering = query.length > 0
  // Trust the server's count, but never claim a category is empty while rows
  // for it are on screen (e.g. the count is momentarily stale after a move).
  const total = category
    ? Math.max(category.password_count, section.total)
    : section.total
  const shown = filtering ? passwords.length : total
  const emptyMatch = filtering && passwords.length === 0

  if (emptyMatch) return null

  return (
    <section>
      <div className="flex flex-wrap items-center gap-2 bg-canvas-subtle px-3 py-1.5">
        <TagIcon size={12} aria-hidden="true" className="shrink-0 text-muted" />
        <h3 className="text-xs font-semibold">
          {category ? category.name : 'Uncategorized'}
        </h3>
        <Label
          variant="muted"
          title={
            filtering && shown !== total
              ? `${shown} of ${total} shown`
              : `${total} ${total === 1 ? 'app password' : 'app passwords'}`
          }
        >
          {filtering && shown !== total ? `${shown} of ${total}` : total}
        </Label>
        {category && total === 0 ? (
          <span className="text-xs text-muted">Empty</span>
        ) : null}
        <div className="ml-auto flex items-center gap-2">
          {onCreatePasswordHere ? (
            <button
              type="button"
              onClick={onCreatePasswordHere}
              className="rounded px-1.5 py-0.5 text-xs text-muted hover:bg-neutral-subtle hover:text-foreground"
            >
              Add password
            </button>
          ) : null}
          {category ? (
            total === 0 ? (
              <ConfirmAction
                label="Delete category"
                confirmLabel="Delete category"
                resourceName={`the empty category ${category.name}`}
                pending={removingCategory === category.id}
                onConfirm={() => onRemoveCategory(category)}
              />
            ) : (
              <span
                title="Move every app password out of this category before deleting it."
                className="inline-flex cursor-not-allowed items-center gap-1 rounded px-1.5 py-0.5 text-xs text-muted opacity-60"
              >
                <TrashIcon size={12} aria-hidden="true" />
                Delete
                <span className="sr-only">
                  {' '}
                  category — move its app passwords out first
                </span>
              </span>
            )
          ) : null}
        </div>
      </div>

      {passwords.length === 0 && category ? (
        <p className="px-4 py-2 text-xs text-muted">
          No app passwords in this category yet.
        </p>
      ) : null}

      {passwords.length > 0 ? (
        <ul>
          {passwords.map((appPassword) => (
            <AppPasswordCard
              key={appPassword.id}
              appPassword={appPassword}
              categories={categories}
              expanded={expandedId === appPassword.id}
              onToggle={() => onTogglePassword(appPassword.id)}
              pendingToken={
                issued && issued.appPasswordId === appPassword.id
                  ? issued.token
                  : null
              }
              onTokenDismiss={onTokenDismiss}
              onChanged={onChanged}
              onToken={(token, label) => onToken(appPassword.id, token, label)}
            />
          ))}
        </ul>
      ) : null}
    </section>
  )
}

type CategoryCreatorProps = {
  id: string
  value: string
  onChange: (value: string) => void
  onSubmit: (event: FormEvent<HTMLFormElement>) => void
  pending: boolean
}

/** Always-visible "new category" row at the foot of the list. */
function CategoryCreator({
  id,
  value,
  onChange,
  onSubmit,
  pending,
}: CategoryCreatorProps) {
  return (
    <form
      onSubmit={onSubmit}
      className="flex flex-wrap items-center gap-2 px-3 py-2"
    >
      <label htmlFor={id} className="sr-only">
        New category name
      </label>
      <div className="w-56">
        <input
          id={id}
          value={value}
          onChange={(event) => onChange(event.target.value)}
          placeholder="New category"
          maxLength={40}
          className={compactInputClasses('h-7')}
        />
      </div>
      <Button size="sm" type="submit" disabled={pending || value.trim().length === 0}>
        {pending ? 'Adding…' : 'Add category'}
      </Button>
      <span className="text-xs text-muted">
        Group related app passwords.
      </span>
    </form>
  )
}
