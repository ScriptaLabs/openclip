; OpenClip installer hooks (Tauri NSIS).
;
; The installer is per-user, so it never asks for administrator rights. That also keeps automatic
; updates fully silent. Launching with Windows is on by default for a fresh install, because OpenClip
; only records while it is running; the choice is remembered, so updates never undo it.

!macro NSIS_HOOK_POSTINSTALL
  IfFileExists "$APPDATA\dev.openclip.app\autostart.initialized" openclip_autostart_done
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "OpenClip" '"$INSTDIR\OpenClip.exe" --background'
    CreateDirectory "$APPDATA\dev.openclip.app"
    FileOpen $0 "$APPDATA\dev.openclip.app\autostart.initialized" w
    FileClose $0
  openclip_autostart_done:
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; The in-app "Launch with Windows" switch writes this value; remove it with the app.
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "OpenClip"
!macroend
