; Clean up stale desktop / start-menu shortcuts so reinstall or rename
; paths do not leave "Cockpit Tools (2)" style duplicates on the desktop.
; Tauri's CreateOrUpdateDesktopShortcut already skips creation in /UPDATE mode;
; this hook covers manual reinstalls and historical product-name changes.

!macro NSIS_HOOK_PREINSTALL
  ; Current product name
  Delete "$DESKTOP\${PRODUCTNAME}.lnk"
  Delete "$SMPROGRAMS\${PRODUCTNAME}.lnk"

  ; Historical / alternate display names that may have left shortcuts behind
  Delete "$DESKTOP\Cockpit Tools.lnk"
  Delete "$DESKTOP\Antigravity Cockpit Tools.lnk"
  Delete "$DESKTOP\Antigravity Cockpit.lnk"
  Delete "$SMPROGRAMS\Cockpit Tools.lnk"
  Delete "$SMPROGRAMS\Antigravity Cockpit Tools.lnk"
  Delete "$SMPROGRAMS\Antigravity Cockpit.lnk"
!macroend
