import { Crosshair, X } from "lucide-react"
import {
  DialogTrigger,
  ModalOverlay,
  type ModalOverlayProps,
  Dialog as RADialog,
  Modal as RAModal,
} from "react-aria-components"
import { tv } from "tailwind-variants"
import { Button } from "./button"

const dialog = tv({
  slots: {
    overlay:
      "data-[entering]:fade-in data-[exiting]:fade-out fixed inset-0 z-50 flex items-center justify-center bg-background-overlay data-[entering]:animate-in data-[exiting]:animate-out",
    modal:
      "data-[entering]:zoom-in-95 data-[exiting]:zoom-out-95 relative w-full max-w-[560px] rounded-[14px] border border-border bg-background-elevated text-foreground shadow-2xl data-[entering]:animate-in data-[exiting]:animate-out",
    header: "flex items-center justify-end px-2.5 pt-2.5",
    closeButton:
      "inline-flex h-7 w-7 items-center justify-center rounded-md bg-background-glass text-foreground-subtle hover:bg-background-glass-medium",
    body: "flex flex-col items-center justify-center gap-3.5 px-6 pt-4 pb-6",
    footer:
      "flex h-16 items-center justify-end gap-3 border-border border-t px-6",
    iconWrapper:
      "flex h-[72px] w-[72px] items-center justify-center rounded-full bg-background-glass-medium",
    description: "text-center text-[15px] text-foreground-muted",
    hint: "flex h-8 items-center gap-2 rounded-md border border-border-muted bg-background-card px-3",
    keyBadge:
      "inline-flex h-5 min-w-[28px] items-center justify-center rounded bg-background-subtle px-1.5 font-semibold text-[10px] text-foreground",
    hintText: "text-[13px] text-foreground-subtle",
  },
})

export interface DialogProps {
  children: React.ReactNode
  isOpen?: boolean
  onOpenChange?: (isOpen: boolean) => void
}

/**
 * A modal dialog component styled according to the design system.
 *
 * @example
 * ```tsx
 * <Dialog>
 *   <DialogTrigger>
 *     <Button>Open Dialog</Button>
 *   </DialogTrigger>
 *   <DialogContent>
 *     <DialogHeader>
 *       <DialogClose />
 *     </DialogHeader>
 *     <DialogBody>
 *       <p>Dialog content goes here.</p>
 *     </DialogBody>
 *     <DialogFooter>
 *       <Button variant="outline">Cancel</Button>
 *       <Button>Confirm</Button>
 *     </DialogFooter>
 *   </DialogContent>
 * </Dialog>
 * ```
 */
export function Dialog({ children, isOpen, onOpenChange }: DialogProps) {
  return (
    <DialogTrigger isOpen={isOpen} onOpenChange={onOpenChange}>
      {children}
    </DialogTrigger>
  )
}

const {
  overlay,
  modal,
  header,
  closeButton,
  body,
  footer,
  iconWrapper,
  description,
  hint,
  keyBadge,
  hintText,
} = dialog()

export interface DialogContentProps extends ModalOverlayProps {
  children: React.ReactNode
}

/**
 * The content container for the dialog.
 * Must be used inside a `Dialog` component.
 */
export function DialogContent({ children, ...props }: DialogContentProps) {
  return (
    <ModalOverlay className={overlay()} {...props}>
      <RAModal className={modal()}>
        <RADialog className="outline-none">{children}</RADialog>
      </RAModal>
    </ModalOverlay>
  )
}

export interface DialogHeaderProps
  extends React.HTMLAttributes<HTMLDivElement> {
  children?: React.ReactNode
}

/**
 * Header section of the dialog.
 * Typically contains a close button or title.
 */
export function DialogHeader({
  children,
  className,
  ...props
}: DialogHeaderProps) {
  return (
    <div className={header({ className })} {...props}>
      {children}
    </div>
  )
}

export interface DialogCloseProps {
  className?: string
  onPress?: () => void
}

/**
 * Close button for the dialog.
 * Uses the X icon from Lucide.
 */
export function DialogClose({ className, onPress }: DialogCloseProps) {
  return (
    <Button
      className={closeButton({ className })}
      onPress={onPress}
      aria-label="Close dialog"
    >
      <X className="h-3.5 w-3.5" />
    </Button>
  )
}

export interface DialogBodyProps extends React.HTMLAttributes<HTMLDivElement> {
  children: React.ReactNode
}

/**
 * Body section of the dialog.
 * Contains the main content.
 */
export function DialogBody({ children, className, ...props }: DialogBodyProps) {
  return (
    <div className={body({ className })} {...props}>
      {children}
    </div>
  )
}

export interface DialogFooterProps
  extends React.HTMLAttributes<HTMLDivElement> {
  children: React.ReactNode
}

/**
 * Footer section of the dialog.
 * Typically contains action buttons.
 */
export function DialogFooter({
  children,
  className,
  ...props
}: DialogFooterProps) {
  return (
    <div className={footer({ className })} {...props}>
      {children}
    </div>
  )
}

export interface DialogIconProps extends React.HTMLAttributes<HTMLDivElement> {
  children: React.ReactNode
}

/**
 * Icon wrapper for centered icons in the dialog body.
 * Provides a circular background with medium glass effect.
 */
export function DialogIcon({ children, className, ...props }: DialogIconProps) {
  return (
    <div className={iconWrapper({ className })} {...props}>
      {children}
    </div>
  )
}

export interface DialogDescriptionProps
  extends React.HTMLAttributes<HTMLParagraphElement> {
  children: React.ReactNode
}

/**
 * Description text for the dialog.
 * Centered, muted text styling.
 */
export function DialogDescription({
  children,
  className,
  ...props
}: DialogDescriptionProps) {
  return (
    <p className={description({ className })} {...props}>
      {children}
    </p>
  )
}

export interface DialogHintProps extends React.HTMLAttributes<HTMLDivElement> {
  /** The key combination text to display (e.g., "Esc") */
  keyText?: string
  /** The label text describing what the key does */
  label: string
}

/**
 * Keyboard hint component for dialogs.
 * Shows a key badge with descriptive text.
 *
 * @example
 * ```tsx
 * <DialogHint keyText="Esc" label="Press Esc to cancel" />
 * ```
 */
export function DialogHint({
  keyText,
  label,
  className,
  ...props
}: DialogHintProps) {
  return (
    <div className={hint({ className })} {...props}>
      {keyText && <kbd className={keyBadge()}>{keyText}</kbd>}
      <span className={hintText()}>{label}</span>
    </div>
  )
}

/**
 * Pre-configured pick window dialog following the design from settings.pen.
 * This is a convenience component for the "Pick From Screen" modal.
 *
 * @example
 * ```tsx
 * <PickWindowDialog
 *   isOpen={isOpen}
 *   onOpenChange={setIsOpen}
 *   onClose={() => setIsOpen(false)}
 * />
 * ```
 */
export interface PickWindowDialogProps {
  isOpen: boolean
  onOpenChange: (isOpen: boolean) => void
  onClose?: () => void
}

export function PickWindowDialog({
  isOpen,
  onOpenChange,
  onClose,
}: PickWindowDialogProps) {
  return (
    <Dialog isOpen={isOpen} onOpenChange={onOpenChange}>
      <DialogContent isDismissable>
        <DialogHeader>
          <DialogClose onPress={onClose} />
        </DialogHeader>
        <DialogBody>
          <DialogIcon>
            <Crosshair className="h-[34px] w-[34px] text-white" />
          </DialogIcon>
          <DialogDescription>
            Move your mouse over the app you want to add, then click on it.
          </DialogDescription>
          <DialogHint keyText="Esc" label="Press Esc to cancel" />
        </DialogBody>
      </DialogContent>
    </Dialog>
  )
}

export { DialogTrigger }
