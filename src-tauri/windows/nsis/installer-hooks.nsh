; Clean up historical desktop / start-menu shortcut names during manual installs.
; Tauri skips shortcut creation in /UPDATE mode, so updater runs must preserve
; every existing shortcut. The current product-name shortcut is managed by Tauri.

Var ShortcutRepairRequired

!macro NSIS_HOOK_PREINSTALL
  StrCpy $ShortcutRepairRequired 0

  ; v1.3.15 removed both shortcuts during NSIS updater runs. Repair only that
  ; exact upgrade state so users who intentionally removed one are unaffected.
  ${If} $UpdateMode = 1
    Push $R9
    ReadRegStr $R9 SHCTX "${UNINSTKEY}" "DisplayVersion"
    ${If} $R9 == "1.3.15"
      ${IfNot} ${FileExists} "$DESKTOP\${PRODUCTNAME}.lnk"
        ${IfNot} ${FileExists} "$SMPROGRAMS\${PRODUCTNAME}.lnk"
          StrCpy $ShortcutRepairRequired 1
        ${EndIf}
      ${EndIf}
    ${EndIf}
    Pop $R9
  ${EndIf}

  ${If} $UpdateMode != 1
    ; Historical / alternate display names that may have left shortcuts behind
    Delete "$DESKTOP\Antigravity Cockpit Tools.lnk"
    Delete "$DESKTOP\Antigravity Cockpit.lnk"
    Delete "$SMPROGRAMS\Antigravity Cockpit Tools.lnk"
    Delete "$SMPROGRAMS\Antigravity Cockpit.lnk"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ${If} $ShortcutRepairRequired = 1
    ; Reuse Tauri's shortcut creation functions to keep targets and AppUserModelId consistent.
    StrCpy $UpdateMode 0
    Call CreateOrUpdateStartMenuShortcut
    Call CreateOrUpdateDesktopShortcut
    StrCpy $UpdateMode 1
  ${EndIf}
!macroend
