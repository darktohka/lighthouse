import {
  KeyIcon,
  PlusIcon,
  SearchIcon,
  TagIcon,
  TrashIcon,
} from '@primer/octicons-react'
import { useId, useMemo, useState, type FormEvent } from 'react'

import { isApiError } from '../api/client'
import {
  serviceAccountCategories as categoriesApi,
  serviceAccountList,
} from '../api/endpoints'
import {
  createServiceAccountCategorySchema,
  type ServiceAccount,
  type ServiceAccountCategory,
} from '../api/schemas'
import { validateForm } from '../lib/forms'
import { compactInputClasses } from '../lib/ui'
import { useAsync } from '../lib/useAsync'
import { CreateServiceAccountDialog } from './CreateServiceAccountDialog'
import { OneTimeToken } from './OneTimeToken'
import { ServiceAccountCard } from './ServiceAccountCard'
import { Box, BoxHeader } from './primitives/Box'
import { Button } from './primitives/Button'
import { ConfirmAction } from './ConfirmAction'
import { Flash } from './primitives/Flash'
import { Label } from './primitives/Label'
import { EmptyState, ErrorState, LoadingState } from './primitives/StateViews'

const UNCATEGORIZED_KEY = '__uncategorized__'

type Section = {
  key: string
  category: ServiceAccountCategory | null
  /** Accounts that survive the active search. */
  accounts: ServiceAccount[]
  /** Total before filtering, so a filtered section can say "1 of 3". */
  total: number
}

/** Lowercased haystack of everything the search should look at. */
function searchIndex(
  account: ServiceAccount,
  category: ServiceAccountCategory | null,
): string {
  return [
    account.name,
    account.username,
    account.description ?? '',
    category?.name ?? 'uncategorized',
    ...account.grants.flatMap((grant) => [
      grant.namespace ?? '',
      grant.repository ?? '',
    ]),
    ...account.ip_ranges.map((range) => range.cidr),
  ]
    .join(' ')
    .toLowerCase()
}

export function ServiceAccountsPanel() {
  const accountsState = useAsync(
    (signal) => serviceAccountList.list({ signal }),
    'service-accounts',
  )
  const categoriesState = useAsync(
    (signal) => categoriesApi.list({ signal }),
    'service-account-categories',
  )

  const [query, setQuery] = useState('')
  const [createOpen, setCreateOpen] = useState(false)
  const [createCategoryFor, setCreateCategoryFor] = useState<number | null>(null)
  const [issued, setIssued] = useState<{ token: string; label: string } | null>(null)

  const [newCategoryName, setNewCategoryName] = useState('')
  const [creatingCategory, setCreatingCategory] = useState(false)
  const [categoryError, setCategoryError] = useState<string | null>(null)
  const [removingCategory, setRemovingCategory] = useState<number | null>(null)

  const searchId = useId()
  const newCategoryId = useId()

  const accounts = useMemo(() => accountsState.data ?? [], [accountsState.data])
  const categories = useMemo(
    () => categoriesState.data ?? [],
    [categoriesState.data],
  )

  const reloadAll = () => {
    accountsState.reload()
    categoriesState.reload()
  }

  const createCategory = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    setCategoryError(null)
    const validation = validateForm(createServiceAccountCategorySchema, {
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

  const removeCategory = (category: ServiceAccountCategory) => {
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
              ? `“${category.name}” is not empty — move its accounts first.`
              : error.message
            : 'The category could not be deleted.',
        )
      },
    )
  }

  const normalizedQuery = query.trim().toLowerCase()

  const sections = useMemo<Section[]>(() => {
    const byCategory = new Map<number, ServiceAccount[]>()
    const uncategorized: ServiceAccount[] = []
    const knownCategoryIds = new Set(categories.map((entry) => entry.id))
    for (const account of accounts) {
      // An account pointing at a deleted category falls back to "Uncategorized".
      if (account.category_id === null || !knownCategoryIds.has(account.category_id)) {
        uncategorized.push(account)
      } else {
        const bucket = byCategory.get(account.category_id)
        if (bucket) bucket.push(account)
        else byCategory.set(account.category_id, [account])
      }
    }

    const matches = (account: ServiceAccount, category: ServiceAccountCategory | null) => {
      if (normalizedQuery.length === 0) return true
      return searchIndex(account, category).includes(normalizedQuery)
    }

    const result: Section[] = categories.map((category) => {
      const bucket = byCategory.get(category.id) ?? []
      return {
        key: `category-${category.id}`,
        category,
        accounts: bucket.filter((account) => matches(account, category)),
        total: bucket.length,
      }
    })

    const uncategorizedMatches = uncategorized.filter((account) =>
      matches(account, null),
    )
    if (uncategorizedMatches.length > 0) {
      result.push({
        key: UNCATEGORIZED_KEY,
        category: null,
        accounts: uncategorizedMatches,
        total: uncategorized.length,
      })
    }

    return result
  }, [accounts, categories, normalizedQuery])

  const matchesAny = sections.some((section) => section.accounts.length > 0)
  const totalAccounts = accounts.length

  const loading =
    (accountsState.loading && !accountsState.data) || (categoriesState.loading && !categoriesState.data)
  const error = accountsState.error ?? categoriesState.error

  return (
    <div className="space-y-3">
      {issued ? (
        <OneTimeToken
          token={issued.token}
          title={`Token for ${issued.label}`}
          onDismiss={() => setIssued(null)}
        />
      ) : null}

      <Box>
        <BoxHeader className="gap-2">
          <span className="flex items-center gap-2 text-sm font-medium">
            <KeyIcon size={16} aria-hidden="true" />
            Service accounts
          </span>
          <div className="flex flex-wrap items-center gap-2">
            <div className="relative w-56 sm:w-64">
              <label htmlFor={searchId} className="sr-only">
                Search service accounts
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
                placeholder="Search accounts, grants, IPs…"
                className={compactInputClasses('h-7 pl-7')}
              />
            </div>
            <Button
              size="sm"
              variant="success"
              leadingIcon={<PlusIcon size={14} aria-hidden="true" />}
              onClick={() => setCreateOpen(true)}
            >
              New service account
            </Button>
          </div>
        </BoxHeader>

        {loading ? <LoadingState label="Loading service accounts…" /> : null}
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
                onCreateAccountHere={
                  section.category
                    ? () => setCreateCategoryFor(section.category?.id ?? null)
                    : null
                }
                onChanged={reloadAll}
                onToken={(token, label) => setIssued({ token, label })}
              />
            ))}

            {totalAccounts === 0 && categories.length === 0 ? (
              <EmptyState
                title="No service accounts yet"
                description="Create one to push or pull images from CI without a password."
                icon={<KeyIcon size={24} aria-hidden="true" />}
              />
            ) : null}

            {totalAccounts > 0 && !matchesAny ? (
              <p className="px-4 py-8 text-center text-sm text-muted">
                No service accounts match “{query.trim()}”
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
        <CreateServiceAccountDialog
          key={createCategoryFor === null ? 'new' : `category-${createCategoryFor}`}
          categories={categories}
          initialCategoryId={createCategoryFor}
          onClose={() => {
            setCreateOpen(false)
            setCreateCategoryFor(null)
          }}
          onCreated={(result) => {
            setIssued(result)
            reloadAll()
          }}
        />
      ) : null}
    </div>
  )
}

type CategorySectionProps = {
  section: Section
  categories: ServiceAccountCategory[]
  query: string
  removingCategory: number | null
  onRemoveCategory: (category: ServiceAccountCategory) => void
  onCreateAccountHere: (() => void) | null
  onChanged: () => void
  onToken: (token: string, label: string) => void
}

function CategorySection({
  section,
  categories,
  query,
  removingCategory,
  onRemoveCategory,
  onCreateAccountHere,
  onChanged,
  onToken,
}: CategorySectionProps) {
  const { category, accounts } = section
  const filtering = query.length > 0
  // Trust the server's count, but never claim a category is empty while rows
  // for it are on screen (e.g. the count is momentarily stale after a move).
  const total = category
    ? Math.max(category.account_count, section.total)
    : section.total
  const shown = filtering ? accounts.length : total
  const emptyMatch = filtering && accounts.length === 0

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
              : `${total} ${total === 1 ? 'account' : 'accounts'}`
          }
        >
          {filtering && shown !== total ? `${shown} of ${total}` : total}
        </Label>
        {category && total === 0 ? (
          <span className="text-xs text-muted">Empty</span>
        ) : null}
        <div className="ml-auto flex items-center gap-2">
          {onCreateAccountHere ? (
            <button
              type="button"
              onClick={onCreateAccountHere}
              className="rounded px-1.5 py-0.5 text-xs text-muted hover:bg-neutral-subtle hover:text-foreground"
            >
              Add account
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
                title="Move every account out of this category before deleting it."
                className="inline-flex cursor-not-allowed items-center gap-1 rounded px-1.5 py-0.5 text-xs text-muted opacity-60"
              >
                <TrashIcon size={12} aria-hidden="true" />
                Delete
                <span className="sr-only">
                  {' '}
                  category — move its accounts out first
                </span>
              </span>
            )
          ) : null}
        </div>
      </div>

      {accounts.length === 0 && category ? (
        <p className="px-4 py-2 text-xs text-muted">
          No accounts in this category yet.
        </p>
      ) : null}

      {accounts.length > 0 ? (
        <ul>
          {accounts.map((account) => (
            <ServiceAccountCard
              key={account.id}
              account={account}
              categories={categories}
              onChanged={onChanged}
              onToken={onToken}
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
        Group related accounts.
      </span>
    </form>
  )
}
