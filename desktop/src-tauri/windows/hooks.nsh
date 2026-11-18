; adb, which Orither runs to reach a phone over USB, leaves a server running from
; Orither's own folder after Orither closes. That server holds adb.exe and its DLLs
; open, so an update could not replace them ("Error opening file for writing") and
; an uninstall could not remove them. Only the adb in Orither's folder is stopped;
; another one, such as Android Studio's, is left alone.
!macro ORITHER_STOP_OWN_ADB
  nsExec::Exec `powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "Get-Process adb -ErrorAction SilentlyContinue | Where-Object { $$_.Path -ieq '$INSTDIR\adb\adb.exe' } | Stop-Process -Force; Start-Sleep -Milliseconds 300"`
  Pop $0
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro ORITHER_STOP_OWN_ADB
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro ORITHER_STOP_OWN_ADB
!macroend
