import {
  createFileRoute,
  Link,
  useNavigate,
  useParams,
} from "@tanstack/react-router"
import {
  ArrowLeft,
  Check,
  Crosshair,
  Globe,
  Plus,
  Trash2,
  X,
} from "lucide-react"
import { useEffect, useMemo, useState } from "react"
import { toast } from "sonner"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogBody,
  DialogClose,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogHint,
  DialogIcon,
} from "@/components/ui/dialog"
import { Select, SelectItem } from "@/components/ui/select"
import { TextField } from "@/components/ui/textfield"
import { useConfigDraft } from "@/contexts/config-draft"
import type {
  AppDefinition,
  AppMatcher,
  MatchMethod,
  MatchTarget,
} from "@/types/config"

export const Route = createFileRoute("/applications/$appId/edit")({
  validateSearch: (
    search: Record<string, unknown>,
  ): { pickStep?: "pick" | "select" } => {
    const pickStep =
      search.pickStep === "pick" || search.pickStep === "select"
        ? search.pickStep
        : undefined
    return { pickStep }
  },
  component: AppEditPage,
})

type MatchCondition = {
  id: number
  field: "process_name" | "window_class" | "window_title"
  method: MatchMethod
  value: string
}

function toConditions(matchers: AppMatcher[]): MatchCondition[] {
  return matchers.map((matcher, index) => ({
    id: index + 1,
    field:
      matcher.target === "title"
        ? "window_title"
        : (matcher.target as "process_name" | "window_class"),
    method: matcher.method,
    value: matcher.value,
  }))
}

function toMatcherTarget(field: MatchCondition["field"]): MatchTarget {
  return field === "window_title" ? "title" : field
}

/**
 * App Edit Page - Edit application matching conditions
 * Based on Pencil: "Applications Settings - App Edit"
 */
function AppEditPage() {
  const { appId } = useParams({ from: "/applications/$appId/edit" })
  const search = Route.useSearch()
  const navigate = useNavigate()
  const { draft, setDraft, save, reset, isDirty, isSaving } = useConfigDraft()
  const appIds = useMemo(() => Object.keys(draft.bindings), [draft.bindings])
  const isDefaultApp = appId === "default"
  const currentMatchers = isDefaultApp
    ? []
    : (draft.apps[appId]?.matchers ?? [])

  const [conditions, setConditions] = useState<MatchCondition[]>(
    toConditions(currentMatchers),
  )
  const [activeConditionId, setActiveConditionId] = useState<number | null>(
    null,
  )
  const [selectedDetectKey, setSelectedDetectKey] = useState<
    "process_name" | "window_class" | "window_title"
  >("process_name")
  const [selectedDetectMethod, setSelectedDetectMethod] = useState<
    "exact" | "contains" | "regex"
  >("exact")
  const [editedAppName, setEditedAppName] = useState<string>(appId)

  useEffect(() => {
    if (isDefaultApp) {
      setConditions([])
      setEditedAppName("default")
      return
    }
    setConditions(toConditions(draft.apps[appId]?.matchers ?? []))
    setEditedAppName(draft.apps[appId]?.label ?? "")
  }, [appId, draft.apps, isDefaultApp])

  if (!appIds.includes(appId)) {
    navigate({ to: "/applications" })
    return null
  }

  const pickStep = search.pickStep
  const isPickDialogOpen = pickStep === "pick"
  const isSelectDialogOpen = pickStep === "select"

  const syncConditionsToDraft = (nextConditions: MatchCondition[]) => {
    if (isDefaultApp) return
    setDraft({
      ...draft,
      apps: {
        ...draft.apps,
        [appId]: {
          label: draft.apps[appId]?.label,
          matchers: nextConditions.map((condition) => ({
            target: toMatcherTarget(condition.field),
            method: condition.method,
            value: condition.value,
          })),
        },
      },
    })
  }

  const closePickDialog = () => {
    setActiveConditionId(null)
    navigate({
      to: "/applications/$appId/edit",
      params: { appId },
      search: {},
      replace: true,
    })
  }

  const openPickDialog = (conditionId: number) => {
    setActiveConditionId(conditionId)
    navigate({
      to: "/applications/$appId/edit",
      params: { appId },
      search: { pickStep: "pick" },
    })
  }

  const moveToSelectDialog = () => {
    navigate({
      to: "/applications/$appId/edit",
      params: { appId },
      search: { pickStep: "select" },
      replace: true,
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

  const addCondition = (condition: Omit<MatchCondition, "id">) => {
    const next = [
      ...conditions,
      {
        ...condition,
        id: Math.max(0, ...conditions.map((item) => item.id)) + 1,
      },
    ]
    setConditions(next)
    syncConditionsToDraft(next)
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

  const handleSave = async () => {
    if (!isDefaultApp) {
      const currentApp = draft.apps[appId]
      if (!currentApp) {
        toast.error("Application not found")
        return
      }

      const nextApp: AppDefinition = {
        ...currentApp,
        label: editedAppName.trim(),
      }

      setDraft({
        ...draft,
        apps: {
          ...draft.apps,
          [appId]: nextApp,
        },
      })
    }

    await save()
  }

  return (
    <div className="flex h-full flex-1 flex-col bg-background">
      {/* Header */}
      <div className="flex h-16 items-center justify-between border-border border-b px-6">
        <div className="flex items-center gap-2">
          <div className="flex h-8 w-8 items-center justify-center rounded-md bg-background-subtle">
            <Globe className="h-4 w-4 text-foreground-subtle" />
          </div>
          <TextField
            variant="transparent"
            value={editedAppName}
            onChange={setEditedAppName}
            aria-label="Application name"
            className="w-full max-w-[420px]"
            inputClassName="h-auto font-semibold text-foreground text-lg py-0 px-1"
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

      {/* Body */}
      <div className="flex-1 overflow-y-auto p-6">
        {/* Back Button */}
        <Link
          to="/applications/$appId"
          params={{ appId }}
          className="mb-6 inline-flex h-8 items-center gap-2 rounded-md border bg-transparent px-3 text-[12px] text-foreground-muted transition-colors hover:border-border-bright hover:text-foreground"
        >
          <ArrowLeft className="h-3.5 w-3.5" />
          <span>Back to Gesture Edit</span>
        </Link>

        {/* Section Header */}
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

        {/* Conditions List */}
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
                    onClick={() => {
                      const next = conditions.filter(
                        (item) => item.id !== condition.id,
                      )
                      setConditions(next)
                      syncConditionsToDraft(next)
                    }}
                    className="text-foreground-muted hover:text-foreground"
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                </div>
                <div className="grid gap-3">
                  <Button
                    className="mt-2"
                    variant="outline"
                    onPress={() => openPickDialog(condition.id)}
                    aria-label={`Capture From Screen for condition ${index + 1}`}
                  >
                    <Crosshair className="h-4 w-4" />
                    <span>Capture From Screen</span>
                  </Button>

                  <div className="my-2 h-px flex-1 bg-border" />

                  <div className="grid gap-1.5">
                    <span className="font-medium text-sm">Target</span>
                    <span className="text-foreground-muted text-xs">
                      Which app attribute should be checked.
                    </span>
                    <Select
                      value={condition.field}
                      onChange={(key) =>
                        updateCondition(condition.id, {
                          field: String(key) as MatchCondition["field"],
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
                      <SelectItem id="window_title" textValue="Window Title">
                        Window Title
                      </SelectItem>
                    </Select>
                  </div>

                  <div className="grid gap-1.5">
                    <span className="font-medium text-foreground text-sm">
                      Match
                    </span>
                    <span className="text-foreground-muted text-xs">
                      How the value should be compared.
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

                  <div className="grid gap-1.5">
                    <span className="font-medium text-foreground text-sm">
                      Value
                    </span>
                    <span className="text-foreground-muted text-xs">
                      The actual text to test against the selected target.
                    </span>
                    <input
                      type="text"
                      value={condition.value}
                      onChange={(event) =>
                        updateCondition(condition.id, {
                          value: event.target.value,
                        })
                      }
                      className="h-10 rounded-md border border-border bg-background-card px-3 text-[13px] text-foreground"
                    />
                  </div>
                </div>
              </div>
            ))}
          </div>
          <Button
            variant="outline"
            className="h-10 w-full justify-center gap-2 rounded-lg border-border bg-transparent font-medium text-[13px] text-foreground-muted"
            onPress={() =>
              addCondition({
                field: "process_name",
                method: "exact",
                value: "",
              })
            }
            isDisabled={isDefaultApp}
          >
            <Plus className="h-3.5 w-3.5" />
            <span>Add Condition</span>
          </Button>
        </div>
      </div>

      {/* Footer */}
      <div className="flex h-16 items-center justify-end gap-3 border-border border-t px-6">
        <Link to="/applications/$appId" params={{ appId }}>
          <Button
            variant="outline"
            className="h-9 px-4 text-[13px]"
            onPress={reset}
          >
            Cancel
          </Button>
        </Link>
        <Button
          className="h-9 gap-2 px-4 text-[13px]"
          onPress={handleSave}
          isDisabled={!isDirty || isSaving}
        >
          <span>{isSaving ? "Saving..." : "Save Changes"}</span>
        </Button>
      </div>

      <Dialog
        isOpen={isPickDialogOpen}
        onOpenChange={(isOpen) => !isOpen && closePickDialog()}
      >
        <div />
        <DialogContent
          isDismissable
          onOpenChange={(isOpen) => !isOpen && closePickDialog()}
        >
          <DialogHeader>
            <DialogClose onPress={closePickDialog} />
          </DialogHeader>
          <DialogBody
            className="h-[568px] cursor-crosshair"
            onClick={moveToSelectDialog}
          >
            <DialogIcon>
              <Crosshair className="h-[34px] w-[34px] text-white" />
            </DialogIcon>
            <p className="w-full text-center text-[15px] text-foreground-muted">
              Move your mouse over the app you want to add, then click on it.
            </p>
            <DialogHint keyText="Esc" label="Press Esc to cancel" />
          </DialogBody>
        </DialogContent>
      </Dialog>

      <Dialog
        isOpen={isSelectDialogOpen}
        onOpenChange={(isOpen) => !isOpen && closePickDialog()}
      >
        <div />
        <DialogContent
          isDismissable
          className="bg-background-overlay-light"
          onOpenChange={(isOpen) => !isOpen && closePickDialog()}
        >
          <div className="flex flex-col">
            <div className="flex flex-col gap-3 border-border border-b px-6 pt-5 pb-4">
              <div className="flex items-center justify-between">
                <h3 className="font-semibold text-[16px] text-foreground">
                  App Detected
                </h3>
                <DialogClose onPress={closePickDialog} />
              </div>
              <div className="flex items-center gap-2.5 rounded-lg bg-background px-3 py-2.5">
                <div className="flex h-8 w-8 items-center justify-center rounded-[7px] bg-background-glass-medium">
                  <Globe className="h-[18px] w-[18px] text-white" />
                </div>
                <span className="font-semibold text-[14px] text-foreground">
                  Google Chrome
                </span>
              </div>
            </div>

            <div className="flex flex-col gap-4 px-6 py-5">
              <p className="font-medium text-[13px] text-foreground-subtle">
                Select how to identify this app:
              </p>
              <div className="flex flex-col gap-2">
                <button
                  type="button"
                  onClick={() => setSelectedDetectKey("process_name")}
                  className={`flex w-full items-start gap-3 rounded-lg border p-3.5 text-left ${
                    selectedDetectKey === "process_name"
                      ? "border-border-white bg-background-glass-light"
                      : "border-border bg-transparent"
                  }`}
                >
                  <div
                    className={`mt-[2px] h-4 w-4 rounded-full border ${
                      selectedDetectKey === "process_name"
                        ? "border-4 border-foreground"
                        : "border-[1.5px] border-border-muted"
                    }`}
                  />
                  <div className="flex flex-col gap-1">
                    <p className="font-semibold text-[13px] text-foreground">
                      Process Name
                    </p>
                    <p className="text-foreground-subtle text-sm">
                      Usually stable and recommended
                    </p>
                  </div>
                </button>
                <button
                  type="button"
                  onClick={() => setSelectedDetectKey("window_class")}
                  className={`flex w-full items-start gap-3 rounded-lg border p-3.5 text-left ${
                    selectedDetectKey === "window_class"
                      ? "border-border-white bg-background-glass-light"
                      : "border-border bg-transparent"
                  }`}
                >
                  <div
                    className={`mt-[2px] h-4 w-4 rounded-full border ${
                      selectedDetectKey === "window_class"
                        ? "border-4 border-foreground"
                        : "border-[1.5px] border-border-muted"
                    }`}
                  />
                  <div className="flex flex-col gap-1">
                    <p className="font-semibold text-[13px] text-foreground">
                      Window Class
                    </p>
                    <p className="text-foreground-subtle text-sm">
                      Useful for native windows and terminals
                    </p>
                  </div>
                </button>
                <button
                  type="button"
                  onClick={() => setSelectedDetectKey("window_title")}
                  className={`flex w-full items-start gap-3 rounded-lg border p-3.5 text-left ${
                    selectedDetectKey === "window_title"
                      ? "border-border-white bg-background-glass-light"
                      : "border-border bg-transparent"
                  }`}
                >
                  <div
                    className={`mt-[2px] h-4 w-4 rounded-full border ${
                      selectedDetectKey === "window_title"
                        ? "border-4 border-foreground"
                        : "border-[1.5px] border-border-muted"
                    }`}
                  />
                  <div className="flex flex-col gap-1">
                    <p className="font-semibold text-[13px] text-foreground">
                      Window Title
                    </p>
                    <p className="text-foreground-subtle text-sm">
                      Good for dynamic page-specific matching
                    </p>
                  </div>
                </button>
              </div>
              <div className="flex items-center justify-between gap-3">
                <span className="font-medium text-[13px] text-foreground-subtle">
                  Match method:
                </span>
                <div className="flex items-center gap-1.5">
                  <Button
                    size="sm"
                    variant={
                      selectedDetectMethod === "exact" ? "default" : "outline"
                    }
                    onPress={() => setSelectedDetectMethod("exact")}
                  >
                    Exact
                  </Button>
                  <Button
                    size="sm"
                    variant={
                      selectedDetectMethod === "contains"
                        ? "default"
                        : "outline"
                    }
                    onPress={() => setSelectedDetectMethod("contains")}
                  >
                    Contains
                  </Button>
                  <Button
                    size="sm"
                    variant={
                      selectedDetectMethod === "regex" ? "default" : "outline"
                    }
                    onPress={() => setSelectedDetectMethod("regex")}
                  >
                    Regex
                  </Button>
                </div>
              </div>
            </div>

            <DialogFooter>
              <Button
                variant="outline"
                className="h-9 px-4 text-[13px]"
                onPress={closePickDialog}
              >
                Cancel
              </Button>
              <Button
                className="h-9 gap-2 px-4 text-[13px]"
                onPress={() => {
                  if (activeConditionId !== null) {
                    updateCondition(activeConditionId, {
                      field: selectedDetectKey,
                      method: selectedDetectMethod,
                      value:
                        editedAppName.trim().toLowerCase() || "google-chrome",
                    })
                  }
                  closePickDialog()
                }}
              >
                <Check className="h-3.5 w-3.5" />
                <span>Add This App</span>
              </Button>
            </DialogFooter>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}
