!ifndef ACCESSHUB_BOOTSTRAP_DIRECTORY
  !define ACCESSHUB_BOOTSTRAP_DIRECTORY "$APPDATA\app.accesshub.resellerloader"
!endif

; Input: installer basename on stack. Output: validated ticket or empty string.
; No filename or ticket is written to the installer log.
Function AccessHubParseTicket
  Exch $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  Push $6
  Push $7
  StrCpy $1 ""
  StrCpy $2 $0 17
  StrCmpS $2 "AccessHub-Setup--" 0 accesshub_invalid
  StrCpy $2 $0 4 -4
  StrCmp $2 ".exe" 0 accesshub_invalid
  StrLen $2 $0
  IntCmp $2 64 accesshub_length_ok accesshub_invalid accesshub_length_ok
accesshub_length_ok:
  StrCpy $1 $0 43 17
  StrCpy $7 $0 -4
  StrCpy $7 $7 "" 60
  StrCpy $2 0
accesshub_token_loop:
  StrCpy $3 $1 1 $2
  StrCmp $3 "" accesshub_suffix
  StrCpy $4 "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-"
  StrCpy $5 0
accesshub_character_loop:
  StrCpy $6 $4 1 $5
  StrCmp $6 "" accesshub_invalid
  StrCmpS $3 $6 accesshub_character_ok
  IntOp $5 $5 + 1
  Goto accesshub_character_loop
accesshub_character_ok:
  IntOp $2 $2 + 1
  Goto accesshub_token_loop
accesshub_suffix:
  StrCmp $7 "" accesshub_done
  StrCpy $2 $7 1
  StrCmp $2 " " 0 accesshub_suffix_bracket
  StrCpy $7 $7 "" 1
accesshub_suffix_bracket:
  StrCpy $2 $7 1
  StrCmp $2 "(" 0 accesshub_invalid
  StrCpy $2 $7 1 -1
  StrCmp $2 ")" 0 accesshub_invalid
  StrLen $2 $7
  IntCmp $2 3 accesshub_suffix_digits accesshub_invalid accesshub_suffix_digits
accesshub_suffix_digits:
  StrCpy $7 $7 -1
  StrCpy $7 $7 "" 1
  StrCpy $2 0
accesshub_digit_loop:
  StrCpy $3 $7 1 $2
  StrCmp $3 "" accesshub_done
  StrCpy $4 "0123456789"
  StrCpy $5 0
accesshub_digit_character_loop:
  StrCpy $6 $4 1 $5
  StrCmp $6 "" accesshub_invalid
  StrCmpS $3 $6 accesshub_digit_ok
  IntOp $5 $5 + 1
  Goto accesshub_digit_character_loop
accesshub_digit_ok:
  IntOp $2 $2 + 1
  Goto accesshub_digit_loop
accesshub_invalid:
  StrCpy $1 ""
accesshub_done:
  StrCpy $0 $1
  Pop $7
  Pop $6
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Exch $0
FunctionEnd

!macro NSIS_HOOK_POSTINSTALL
  Push $0
  Push $1
  Push $EXEFILE
  Call AccessHubParseTicket
  Pop $0
  StrCmp $0 "" accesshub_bootstrap_finished
  SetShellVarContext current
  CreateDirectory "${ACCESSHUB_BOOTSTRAP_DIRECTORY}"
  ClearErrors
  FileOpen $1 "${ACCESSHUB_BOOTSTRAP_DIRECTORY}\activation-ticket.tmp" w
  IfErrors accesshub_bootstrap_finished
  FileWrite $1 $0
  FileClose $1
  IfErrors accesshub_bootstrap_cleanup
  Delete "${ACCESSHUB_BOOTSTRAP_DIRECTORY}\activation-ticket"
  Rename "${ACCESSHUB_BOOTSTRAP_DIRECTORY}\activation-ticket.tmp" "${ACCESSHUB_BOOTSTRAP_DIRECTORY}\activation-ticket"
accesshub_bootstrap_cleanup:
  Delete "${ACCESSHUB_BOOTSTRAP_DIRECTORY}\activation-ticket.tmp"
accesshub_bootstrap_finished:
  Pop $1
  Pop $0
!macroend
