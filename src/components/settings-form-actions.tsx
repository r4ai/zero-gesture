import { Button } from "@/components/ui/button"
import { PanelFooter } from "@/components/ui/panel"

export interface SettingsFormActionsProps {
  /**
   * Whether there are unsaved changes
   */
  isDirty: boolean
  /**
   * Whether the form is currently saving
   */
  isSaving: boolean
  /**
   * Callback when the user clicks the Save button
   */
  onSave: () => void
  /**
   * Callback when the user clicks the Cancel button
   */
  onCancel: () => void
}

/**
 * Common action buttons for settings pages
 * Provides Save and Cancel buttons with proper state management
 *
 * @example
 * ```tsx
 * <Panel>
 *   <PanelHeader>...</PanelHeader>
 *   <PanelBody>...</PanelBody>
 *   <SettingsFormActions
 *     isDirty={isDirty}
 *     isSaving={isSaving}
 *     onSave={save}
 *     onCancel={reset}
 *   />
 * </Panel>
 * ```
 */
export function SettingsFormActions({
  isDirty,
  isSaving,
  onSave,
  onCancel,
}: SettingsFormActionsProps) {
  return (
    <PanelFooter>
      <Button variant="outline" onPress={onCancel} isDisabled={!isDirty}>
        Cancel
      </Button>
      <Button onPress={onSave} isDisabled={!isDirty || isSaving}>
        <span className="font-semibold text-[13px]">
          {isSaving ? "Saving..." : "Save Changes"}
        </span>
      </Button>
    </PanelFooter>
  )
}
