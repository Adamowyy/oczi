; Uninstall hooks for the NSIS installer.
;
; The inbox and the log live in %LOCALAPPDATA%\Oczi and are ours, so the
; default uninstaller would leave them behind. The bin\ folder is a leftover
; from the Claude Code relay older builds staged there — uninstalling still
; cleans it up.

!macro NSIS_HOOK_PREUNINSTALL
  RMDir /r "$LOCALAPPDATA\Oczi\bin"
  RMDir /r "$LOCALAPPDATA\Oczi\inbox"
  Delete "$LOCALAPPDATA\Oczi\oczi.log"
!macroend
