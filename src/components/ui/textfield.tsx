import {
  FieldError,
  Input as RAInput,
  Label as RALabel,
  TextField as RATextField,
  type TextFieldProps as RATextFieldProps,
  type ValidationResult,
} from "react-aria-components"

import { tv } from "tailwind-variants"

const textfield = tv({
  slots: {
    label: "mb-1.5 block font-medium text-foreground text-sm",
    inputGroup: "group relative flex items-center",
    input:
      "flex h-10 w-full rounded-md border border-border bg-background px-3 py-2 text-foreground text-sm transition-colors file:border-0 file:bg-transparent file:font-medium file:text-sm placeholder:text-foreground-muted focus-visible:outline-2 focus-visible:outline-foreground focus-visible:outline-offset-2 disabled:cursor-not-allowed disabled:bg-background-muted disabled:opacity-50 group-has-[[slot=end]]:pr-9 group-has-[[slot=start]]:pl-9",
    icon: "pointer-events-none absolute top-1/2 size-5 shrink-0 -translate-y-1/2 text-foreground-muted *:size-full",
    description: "mt-1.5 text-foreground-muted text-sm",
    error: "mt-1.5 font-medium text-destructive text-sm",
  },
  variants: {
    variant: {
      default: {},
      transparent: {
        input:
          "border-transparent bg-transparent focus-visible:ring-0 disabled:bg-transparent disabled:opacity-100",
      },
    },
    iconPosition: {
      start: {
        icon: "left-3",
      },
      end: {
        icon: "right-3",
      },
    },
  },
})

export interface TextFieldProps extends Omit<RATextFieldProps, "children"> {
  label?: string
  description?: string
  errorMessage?: string | ((validation: ValidationResult) => string)
  placeholder?: string
  variant?: "default" | "transparent"
  inputClassName?: string
  children?: React.ReactNode
}

export function TextField({
  label,
  description,
  errorMessage,
  placeholder,
  variant = "default",
  inputClassName,
  className,
  children,
  ...props
}: TextFieldProps) {
  const {
    label: labelClass,
    inputGroup: inputGroupClass,
    input: inputClass,
    description: descriptionClass,
    error: errorClass,
  } = textfield({ variant })

  return (
    <RATextField className={className} {...props}>
      {label && <RALabel className={labelClass()}>{label}</RALabel>}
      <div className={inputGroupClass()}>
        {children}
        <RAInput
          className={inputClass({ className: inputClassName })}
          placeholder={placeholder}
        />
      </div>
      {description && <p className={descriptionClass()}>{description}</p>}
      {errorMessage && (
        <FieldError className={errorClass()}>{errorMessage}</FieldError>
      )}
    </RATextField>
  )
}

TextField.Icon = ({
  slot = "start",
  className: userClassName,
  ...props
}: React.ComponentProps<"div"> & { slot?: "start" | "end" }) => {
  const { icon: iconClass } = textfield({ iconPosition: slot })
  return (
    <div
      {...props}
      slot={slot}
      className={iconClass({ className: userClassName })}
    />
  )
}
