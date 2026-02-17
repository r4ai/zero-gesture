import {
  createFileRoute,
  Link,
  useNavigate,
  useParams,
} from "@tanstack/react-router"
import { ArrowLeft, Check, Globe, Plus, Trash2, X } from "lucide-react"
import { useMemo, useState } from "react"
import { toast } from "sonner"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Select, SelectItem } from "@/components/ui/select"
import { TextField } from "@/components/ui/textfield"
import { useConfigDraft } from "@/contexts/config-draft"
import type { AppMatcher, MatchMethod, MatchTarget } from "@/types/config"

export const Route = createFileRoute("/applications/$appId/edit")({
  component: AppEditPage,
})

type MatchCondition = {
  id: number
  target: MatchTarget
  method: MatchMethod
  value: string
}

function toConditions(matchers: AppMatcher[]): MatchCondition[] {
  return matchers.map((matcher, index) => ({
    id: index + 1,
    target: matcher.target,
    method: matcher.method,
    value: matcher.value,
  }))
}

/**
 * App Edit Page - Edit application matching conditions.
 */
function AppEditPage() {
  const { appId } = useParams({ from: "/applications/$appId/edit" })
  const navigate = useNavigate()
  const { draft, setDraft, isDirty, save, reset, isSaving } = useConfigDraft()
  const appIds = useMemo(() => Object.keys(draft.bindings), [draft.bindings])
  const isDefaultApp = appId === "default"
  const currentMatchers = isDefaultApp
    ? []
    : (draft.apps[appId]?.matchers ?? [])
  const [conditions, setConditions] = useState<MatchCondition[]>(
    toConditions(currentMatchers),
  )
  const [editedAppName, setEditedAppName] = useState<string>(appId)

  if (!appIds.includes(appId)) {
    navigate({ to: "/applications" })
    return null
  }

  const syncConditionsToDraft = (nextConditions: MatchCondition[]) => {
    if (isDefaultApp) return
    setDraft({
      ...draft,
      apps: {
        ...draft.apps,
        [appId]: {
          matchers: nextConditions.map((condition) => ({
            target: condition.target,
            method: condition.method,
            value: condition.value,
          })),
        },
      },
    })
  }

  const updateCondition = (
    conditionId: number,
    patch: Partial<Omit<MatchCondition, "id">>,
  ) => {
    const next = conditions.map((condition) =>
      condition.id === conditionId ? { ...condition, ...patch } : condition,
    )
    setConditions(next)
    syncConditionsToDraft(next)
  }

  const addCondition = () => {
    const next = [
      ...conditions,
      {
        id: Math.max(0, ...conditions.map((item) => item.id)) + 1,
        target: "process_name" as MatchTarget,
        method: "exact" as MatchMethod,
        value: "",
      },
    ]
    setConditions(next)
    syncConditionsToDraft(next)
  }

  const removeCondition = (conditionId: number) => {
    const next = conditions.filter((item) => item.id !== conditionId)
    setConditions(next)
    syncConditionsToDraft(next)
  }

  const renameApp = () => {
    const nextId = editedAppName.trim()
    if (isDefaultApp || nextId.length === 0 || nextId === appId) return
    if (appIds.includes(nextId)) {
      toast.error("Application id already exists")
      return
    }
    setDraft({
      ...draft,
      apps: {
        ...Object.fromEntries(
          Object.entries(draft.apps).filter(([id]) => id !== appId),
        ),
        [nextId]: draft.apps[appId] ?? { matchers: [] },
      },
      bindings: {
        ...Object.fromEntries(
          Object.entries(draft.bindings).filter(([id]) => id !== appId),
        ),
        [nextId]: draft.bindings[appId] ?? [],
      },
    })
    navigate({ to: "/applications/$appId/edit", params: { appId: nextId } })
  }

  const deleteApp = () => {
    if (isDefaultApp) return
    setDraft({
      ...draft,
      apps: Object.fromEntries(
        Object.entries(draft.apps).filter(([id]) => id !== appId),
      ),
      bindings: Object.fromEntries(
        Object.entries(draft.bindings).filter(([id]) => id !== appId),
      ),
    })
    navigate({ to: "/applications" })
  }

  return (
    <div className="flex h-full flex-1 flex-col bg-background">
      <div className="flex h-16 items-center justify-between border-border border-b px-6">
        <div className="flex items-center gap-2">
          <div className="flex h-8 w-8 items-center justify-center rounded-md bg-background-subtle">
            <Globe className="h-4 w-4 text-foreground-subtle" />
          </div>
          <TextField
            variant="transparent"
            value={editedAppName}
            onChange={setEditedAppName}
            onBlur={renameApp}
            aria-label="Application name"
            className="w-full max-w-[420px]"
            inputClassName="h-auto px-1 py-0 font-semibold text-foreground text-lg"
            isDisabled={isDefaultApp}
          />
        </div>
        <Button
          variant="outline"
          className="h-8 gap-2 rounded-md border-destructive-subtle bg-destructive-subtle text-destructive text-sm hover:bg-destructive/20"
          onPress={deleteApp}
          isDisabled={isDefaultApp}
        >
          <Trash2 className="h-3.5 w-3.5" />
          <span>Delete App</span>
        </Button>
      </div>

      <div className="flex-1 overflow-y-auto p-6">
        <Link
          to="/applications/$appId"
          params={{ appId }}
          className="mb-6 inline-flex h-8 items-center gap-2 rounded-md border bg-transparent px-3 text-[12px] text-foreground-muted transition-colors hover:border-border-bright hover:text-foreground"
        >
          <ArrowLeft className="h-3.5 w-3.5" />
          <span>Back to Gesture Edit</span>
        </Link>

        <div className="mb-6 flex items-center justify-between">
          <div className="flex flex-col gap-1">
            <h3 className="font-semibold text-foreground">
              Matching Conditions
            </h3>
            <p className="text-foreground-muted text-sm">
              App matches when ANY condition is met
            </p>
          </div>
        </div>

        {isDefaultApp ? (
          <div className="rounded-lg border border-border bg-background-elevated p-4 text-foreground-muted text-sm">
            default app is fallback and cannot have matchers.
          </div>
        ) : (
          <div className="grid gap-3">
            <div className="grid grid-cols-1 gap-3">
              {conditions.map((condition, index) => (
                <div
                  key={condition.id}
                  className="flex flex-col gap-3 rounded-lg border border-border bg-background-elevated p-4"
                >
                  <div className="flex items-center justify-between">
                    <Badge variant="success" className="text-xs">
                      Condition {index + 1}
                    </Badge>
                    <button
                      type="button"
                      onClick={() => removeCondition(condition.id)}
                      className="text-foreground-muted hover:text-foreground"
                    >
                      <X className="h-3.5 w-3.5" />
                    </button>
                  </div>
                  <div className="grid gap-3">
                    <div className="grid gap-1.5">
                      <span className="font-medium text-sm">Target</span>
                      <Select
                        value={condition.target}
                        onChange={(key) =>
                          updateCondition(condition.id, {
                            target: String(key) as MatchTarget,
                          })
                        }
                        className="w-full"
                      >
                        <SelectItem id="process_name" textValue="Process Name">
                          Process Name
                        </SelectItem>
                        <SelectItem id="window_class" textValue="Window Class">
                          Window Class
                        </SelectItem>
                        <SelectItem id="title" textValue="Window Title">
                          Window Title
                        </SelectItem>
                      </Select>
                    </div>

                    <div className="grid gap-1.5">
                      <span className="font-medium text-foreground text-sm">
                        Match
                      </span>
                      <Select
                        value={condition.method}
                        onChange={(key) =>
                          updateCondition(condition.id, {
                            method: String(key) as MatchMethod,
                          })
                        }
                        className="w-full"
                      >
                        <SelectItem id="exact" textValue="exact">
                          exact
                        </SelectItem>
                        <SelectItem id="contains" textValue="contains">
                          contains
                        </SelectItem>
                        <SelectItem id="regex" textValue="regex">
                          regex
                        </SelectItem>
                      </Select>
                    </div>

                    <TextField
                      value={condition.value}
                      onChange={(value) =>
                        updateCondition(condition.id, { value })
                      }
                      className="w-full"
                      aria-label={`Condition ${index + 1} value`}
                    />
                  </div>
                </div>
              ))}
            </div>
            <Button
              variant="outline"
              className="h-10 w-full justify-center gap-2 rounded-lg border-border bg-transparent font-medium text-[13px] text-foreground-muted"
              onPress={addCondition}
            >
              <Plus className="h-3.5 w-3.5" />
              <span>Add Condition</span>
            </Button>
          </div>
        )}
      </div>

      <div className="flex h-16 items-center justify-end gap-3 border-border border-t px-6">
        <Button
          variant="outline"
          className="h-9 px-4 text-[13px]"
          onPress={reset}
        >
          Cancel
        </Button>
        <Button
          className="h-9 gap-2 px-4 text-[13px]"
          onPress={save}
          isDisabled={!isDirty || isSaving}
        >
          <Check className="h-3.5 w-3.5" />
          <span>{isSaving ? "Saving..." : "Save Changes"}</span>
        </Button>
      </div>
    </div>
  )
}
