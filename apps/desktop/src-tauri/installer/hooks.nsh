; Extra steps of the NSIS installer (tauri.conf.json: bundle.windows.nsis.installerHooks).

; Runs at the end of the uninstaller.
!macro NSIS_HOOK_POSTUNINSTALL
  ; "Start Symphony when I sign in" is a per-user registry entry the app writes itself, so the
  ; uninstaller does not know about it. Left behind, it would try to launch a program that is gone
  ; at every sign-in. (An upgrade runs this too; the app registers the entry again on its next
  ; start while the setting is on.)
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "SymphonyDesktopRuntime"
!macroend
