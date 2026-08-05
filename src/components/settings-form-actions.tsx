import { Button } from "@/components/ui/button"
import { PanelFooter } from "@/components/ui/panel"
import { useConfigDraft } from "@/contexts/config-draft"

/**
 * Common action buttons for settings pages
 * Provides Save and Cancel buttons with proper state management
 *
 * @example
 * ```tsx
 * <Panel>
 *   <PanelHeader>...</PanelHeader>
 *   <PanelBody>...</PanelBody>
 *   <SettingsFormActions />
 * </Panel>
 * ```
 */
export function SettingsFormActions() {
  const { isDirty, isSaving, save, reset, saveError } = useConfigDraft()

  return (
    <PanelFooter>
      {saveError ? (
        <p className="mr-auto text-red-600 text-xs" role="alert">
          {saveError}
        </p>
      ) : null}
      <Button variant="outline" onPress={reset} isDisabled={!isDirty}>
        Cancel
      </Button>
      <Button onPress={save} isDisabled={!isDirty || isSaving}>
        <span className="font-semibold text-[13px]">
          {isSaving ? "Saving..." : "Save Changes"}
        </span>
      </Button>
    </PanelFooter>
  )
}
