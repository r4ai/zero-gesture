!macro NSIS_HOOK_PREUNINSTALL
  ReadEnvStr $0 "ZG_P05C_INSTALLED_ACCEPTANCE"
  StrCmp $0 "disposable-runner" 0 p05c_continue_uninstall
  ReadEnvStr $0 "ZG_P05C_ABORT_UNINSTALL"
  StrCmp $0 "disposable-runner" 0 p05c_continue_uninstall
  Abort "P05c disposable acceptance requested uninstall cancellation."

  p05c_continue_uninstall:
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Zero Gesture"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "Zero Gesture"
!macroend
