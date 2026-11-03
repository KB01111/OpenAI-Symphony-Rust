; Extra steps of the NSIS installer (tauri.conf.json: bundle.windows.nsis.installerHooks).

; Runs at the end of the uninstaller.
!macro NSIS_HOOK_POSTUNINSTALL
  ; "Start Symphony when I sign in" is a per-user registry entry the app writes itself, so the
  ; uninstaller does not know about it. Left behind, it would try to launch a program that is gone
  ; at every sign-in. (An upgrade runs this too; the app registers the entry again on its next
  ; start while the setting is on.)
  ;
  ; All copies of the app share this one value, so it is removed only when it launches a program
  ; inside the installation being removed ("<install dir>\...). An entry that belongs to another
  ; copy (a development build, a second install) is left to that copy.
  Push $0
  Push $1
  Push $2
  Push $3
  ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "SymphonyDesktopRuntime"
  ${If} $0 != ""
    StrCpy $1 '"$INSTDIR\'
    StrLen $2 $1
    StrCpy $3 $0 $2
    ; Compared without regard to case, as Windows paths are.
    ${If} $3 == $1
      DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "SymphonyDesktopRuntime"
    ${EndIf}
  ${EndIf}
  Pop $3
  Pop $2
  Pop $1
  Pop $0
!macroend
