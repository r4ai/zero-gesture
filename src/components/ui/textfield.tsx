import {
  FieldError,
  Input as RAInput,
  Label as RALabel,
  TextField as RATextField,
  type TextFieldProps as RATextFieldProps,
  type ValidationResult,
} from "react-aria-components"

import { tv } from "tailwind-variants"

const input = tv({
  base: "flex h-10 w-full rounded-md border border-border bg-background px-3 py-2 text-sm ring-offset-background file:border-0 file:bg-transparent file:font-medium file:text-sm placeholder:text-foreground-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50",
})

export interface TextFieldProps extends RATextFieldProps {
  label?: string
  description?: string
  errorMessage?: string | ((validation: ValidationResult) => string)
  placeholder?: string
  startIcon?: React.ReactNode
}

export function TextField({
  label,
  description,
  errorMessage,
  placeholder,
  startIcon,
  className,
  ...props
}: TextFieldProps) {
  return (
    <RATextField className={className} {...props}>
      {label && <RALabel>{label}</RALabel>}
      <div className="relative">
        {startIcon && (
          <div className="absolute top-2.5 left-2.5 h-4 w-4 text-foreground-muted">
            {startIcon}
          </div>
        )}
        <RAInput
          className={input({ className: startIcon ? "pl-9" : "" })}
          placeholder={placeholder}
        />
      </div>
      {description && (
        <p className="text-foreground-muted text-sm">{description}</p>
      )}
      {errorMessage && (
        <FieldError className="font-medium text-destructive text-sm">
          {errorMessage}
        </FieldError>
      )}
    </RATextField>
  )
}
