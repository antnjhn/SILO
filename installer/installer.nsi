; ===========================================================================
;  SILO SETUP  -  standalone installer
; ---------------------------------------------------------------------------
;  Layout mirrors the SILO installer mockup: a left rail holding the mark and
;  the "Every game. Its own place." tagline, a right column with a tracked
;  kicker, a large heading, supporting copy, and a footer line above the
;  action buttons.
;
;  Independent of SILO's own packaging:
;    * it does not use or modify src-tauri/installer-template.nsi
;    * it does not modify any file under src / src-tauri
;    * it installs per-user (no admin) and never reads, moves or deletes the
;      SILO library data in %APPDATA%\com.silo.launcher
;
;  Build:  .\build.ps1
;
;  nsDialogs quirks this script works around:
;   * images (logo, toggle) are created BEFORE the black backdrop, text and
;     flat buttons AFTER it - otherwise Windows paints the backdrop over them;
;   * a real BUTTON cannot be filled on a page, so actions are flat statics
;     with SS_NOTIFY (a click still arrives as BN_CLICKED, so NSD_OnClick
;     drives them like buttons);
;   * the stock Next/Back/Cancel buttons stay in the window (hidden) so Enter
;     and Escape keep working.
; ===========================================================================

Unicode true

!include "nsDialogs.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

; --- Identity --------------------------------------------------------------
!define APP_NAME    "SILO"
!define APP_VERSION "0.5.1"
!define APP_ID      "com.silo.launcher"
!define APP_EXE     "silo.exe"
!define SRC_EXE     "..\src-tauri\target\release\silo.exe"
!define SRC_ICO     "..\src-tauri\icons\icon.ico"
!define SETUP_ICO   "assets\installer-icon.ico"
!define SRC_LICENSE "..\LICENSE"



; --- Design tokens (0xBBGGRR) ---------------------------------------------
!define BG          0x000000
!define HAIRLINE    0x1A1A1A
!define PANEL       0x17171C
!define TEXT        0xF2F2F5
!define TEXT_SUB    0xC4C4CC
!define BODY        0x9A9AA2
!define LABEL       0x8E8E96
!define FAINT       0x6B6B72
!define ACCENT      0x7C5CFA   ; #7C5CFA
!define ACCENT_INK  0xFFFFFF
!define BTN_DARK    0x1A1A20

!define PAGE_W    760
!define PAGE_H    520
!define RAIL_W    250
!define CX        288
!define CW        386
!define BTN_Y_OFF 58
!define BTN_H     40

; --- Installer window ------------------------------------------------------
Name "${APP_NAME}"
Caption "${APP_NAME} Setup"
UninstallCaption "${APP_NAME} Uninstall"
OutFile "dist\SILO-Setup-${APP_VERSION}.exe"
InstallDir "$LOCALAPPDATA\Programs\SILO"
InstallDirRegKey HKCU "Software\${APP_ID}" "InstallDir"
RequestExecutionLevel user
ShowInstDetails nevershow
ShowUninstDetails nevershow
BrandingText " "
SetCompressor /SOLID lzma
XPStyle on
ManifestDPIAware true
Icon "${SETUP_ICO}"
UninstallIcon "${SETUP_ICO}"
WindowIcon on

VIProductVersion "0.5.1.0"
VIAddVersionKey /LANG=1033 "ProductName"     "${APP_NAME}"
VIAddVersionKey /LANG=1033 "FileDescription" "${APP_NAME} Setup"
VIAddVersionKey /LANG=1033 "FileVersion"     "${APP_VERSION}"
VIAddVersionKey /LANG=1033 "ProductVersion"  "${APP_VERSION}"
VIAddVersionKey /LANG=1033 "CompanyName"     "Anton John"
VIAddVersionKey /LANG=1033 "LegalCopyright"  "Copyright (c) 2026 Anton John"

; --- Pages -----------------------------------------------------------------
Page custom WelcomePage
Page custom LicensePage
Page custom LocationPage
Page custom ProgressPage
Page custom FinishPage

UninstPage custom un.IntroPage
UninstPage custom un.ConfirmPage
UninstPage custom un.ProgressPage
UninstPage custom un.DonePage

; --- Variables -------------------------------------------------------------
Var PageDialog
Var PageW
Var PageH
Var BtnY
Var FootY
Var PriX
Var SecX
Var ImageHandle
Var ToggleImage
Var LogoBox
Var CheckBox
Var hPathField
Var hFill
Var bk_w
Var bk_h

Var hLicense
Var LicenseAccepted
Var LaunchOn
Var StepVar
Var FontH1
Var FontSub
Var FontBody
Var FontMicro
Var FontBtn
Var FontTag

; --- Macros ----------------------------------------------------------------
!macro MKFONT outVar px weight
  System::Call 'gdi32::CreateFontW(i -${px}, i 0, i 0, i 0, i ${weight}, i 0, i 0, i 0, i 0, i 0, i 0, i 5, i 0, w "Segoe UI") p.r0'
  StrCpy ${outVar} $0
!macroend

; Flat button: a static painted in token colours that still reports clicks.
!macro FLATBTN x y w h txt fg bg handler
  ${NSD_CreateLabel} ${x} ${y} ${w} ${h} "${txt}"
  Pop $0
  ${NSD_AddStyle} $0 ${SS_NOTIFY}
  ${NSD_AddStyle} $0 ${SS_CENTER}
  ${NSD_AddStyle} $0 ${SS_CENTERIMAGE}
  SetCtlColors $0 ${fg} ${bg}
  SendMessage $0 0x0030 $FontBtn 1
  ; nsDialogs marks labels as WS_EX_TRANSPARENT, which makes real mouse clicks
  ; pass straight through them; clear that bit so the button can be clicked
  System::Call 'user32::GetWindowLongW(p $0, i -20) i.r1'
  IntOp $1 $1 & 0xFFFFFFDF
  System::Call 'user32::SetWindowLongW(p $0, i -20, i r1)'
  ; the backdrop label is transparent and stays above newer controls, so lift the
  ; button to the top of the z-order as well as out of the click-through state
  System::Call 'user32::SetWindowPos(p $0, p 0, i 0, i 0, i 0, i 0, i 0x0013)'
  ; round the corners the way the mockup does
  IntOp $2 ${w} + 1
  IntOp $3 ${h} + 1
  System::Call 'gdi32::CreateRoundRectRgn(i 0, i 0, i r2, i r3, i 18, i 18) p.r6'
  ${If} $6 != 0
    System::Call 'user32::SetWindowRgn(p $0, p r6, i 1)'
  ${EndIf}
  ${NSD_OnClick} $0 ${handler}
!macroend

; Tracked micro label.
!macro MICRO txt x y w
  ${NSD_CreateLabel} ${x} ${y} ${w} 14 "${txt}"
  Pop $0
  SendMessage $0 0x0030 $FontMicro 1
  SetCtlColors $0 ${LABEL} ${BG}
!macroend

; Page heading block: kicker, big heading, subtitle.
!macro HEAD kick head sub
  !insertmacro MICRO "${kick}" ${CX} 84 400

  ${NSD_CreateLabel} ${CX} 102 ${CW} 42 "${head}"
  Pop $0
  SendMessage $0 0x0030 $FontH1 1
  SetCtlColors $0 ${TEXT} ${BG}

  ${NSD_CreateLabel} ${CX} 150 ${CW} 20 "${sub}"
  Pop $0
  SendMessage $0 0x0030 $FontSub 1
  SetCtlColors $0 ${TEXT_SUB} ${BG}
!macroend

; Body copy line.
!macro BODY txt x y w
  ${NSD_CreateLabel} ${x} ${y} ${w} 20 "${txt}"
  Pop $0
  SendMessage $0 0x0030 $FontBody 1
  SetCtlColors $0 ${BODY} ${BG}
!macroend

; Footer micro line, bottom-left of the content column.
!macro FOOT txt
  !insertmacro MICRO "${txt}" ${CX} $FootY 240
!macroend

; Vertical rail divider.
!macro RAILRULE
  ${NSD_CreateLabel} ${RAIL_W} 0 1 $PageH ""
  Pop $0
  SetCtlColors $0 ${HAIRLINE} ${HAIRLINE}
!macroend

; --- Init ------------------------------------------------------------------
Function .onInit
  InitPluginsDir
  File /oname=$PLUGINSDIR\rail.bmp     "gen\rail_bg.bmp"
  File /oname=$PLUGINSDIR\check_on.bmp  "gen\check_on.bmp"
  File /oname=$PLUGINSDIR\check_off.bmp "gen\check_off.bmp"
  File /oname=$PLUGINSDIR\LICENSE.txt   "${SRC_LICENSE}"

  !insertmacro MKFONT $FontH1    32 400
  !insertmacro MKFONT $FontSub   14 400
  !insertmacro MKFONT $FontBody  13 400
  !insertmacro MKFONT $FontMicro 10 600
  !insertmacro MKFONT $FontBtn   13 600
  !insertmacro MKFONT $FontTag   12 400

  StrCpy $LicenseAccepted "0"
  StrCpy $LaunchOn "1"
FunctionEnd

; --- Page setup ------------------------------------------------------------
Function PageFrame
  nsDialogs::Create 1018
  Pop $PageDialog
  ${If} $PageDialog != error
    System::Call 'gdi32::CreateSolidBrush(i 0) p.r8'
    ${If} $8 != 0
      System::Call 'user32::SetClassLongPtrW(p $PageDialog, i -10, p r8)'
    ${EndIf}
  ${EndIf}
  ${If} $PageDialog == error
    Abort
  ${EndIf}

  ; a fixed-size window: the invisible resize border is what shows as pale bars
  ; along the right and bottom edges, and the mockup has no maximize button
  System::Call 'user32::GetWindowLongW(p $HWNDPARENT, i -16) i.r1'
  IntOp $1 $1 & 0xFFFAFFFF   ; drop WS_THICKFRAME | WS_MAXIMIZEBOX
  System::Call 'user32::SetWindowLongW(p $HWNDPARENT, i -16, i r1)'
  System::Call 'user32::SetWindowPos(p $HWNDPARENT, i 0, i 0, i 0, i 0, i 0, i 0x0027)'

  System::Call 'user32::GetSystemMetrics(i 0) i.r0'
  System::Call 'user32::GetSystemMetrics(i 1) i.r1'
  IntOp $2 $0 - ${PAGE_W}
  IntOp $3 $1 - ${PAGE_H}
  IntOp $2 $2 / 2
  IntOp $3 $3 / 2
  System::Call 'user32::SetWindowPos(i $HWNDPARENT, i 0, i $2, i $3, i ${PAGE_W}, i ${PAGE_H}, i 0x0004)'

  ; Stock buttons stay available for Enter/Escape but are not drawn.
  GetDlgItem $9 $HWNDPARENT 1
  System::Call 'user32::ShowWindow(p r9, i 0)'
  GetDlgItem $9 $HWNDPARENT 2
  System::Call 'user32::ShowWindow(p r9, i 0)'
  GetDlgItem $9 $HWNDPARENT 3
  System::Call 'user32::ShowWindow(p r9, i 0)'

  System::Call 'dwmapi::DwmSetWindowAttribute(p $HWNDPARENT, i 20, *i 1, i 4)'
  System::Call 'dwmapi::DwmSetWindowAttribute(p $HWNDPARENT, i 19, *i 1, i 4)'
  System::Call 'dwmapi::DwmSetWindowAttribute(p $HWNDPARENT, i 34, *i 0, i 4)'
  System::Call 'dwmapi::DwmSetWindowAttribute(p $HWNDPARENT, i 35, *i 0, i 4)'
  System::Call 'user32::SetForegroundWindow(p $HWNDPARENT)'
  System::Call 'user32::SetActiveWindow(p $HWNDPARENT)'

  System::Call 'gdi32::CreateSolidBrush(i 0x000000) p.r8'
  ${If} $8 != 0
    System::Call 'user32::SetClassLongPtrW(p $HWNDPARENT, i -10, p r8)'
  ${EndIf}

  System::Alloc 16
  Pop $1
  System::Call 'user32::GetClientRect(p $HWNDPARENT, p r1)'
  System::Call '*$1(i.r2, i.r3, i.r4, i.r5)'
  System::Free $1
  StrCpy $PageW $4
  StrCpy $PageH $5

  IntOp $6 $4 + 2
  IntOp $7 $5 + 2
  System::Call 'user32::SetWindowPos(p $PageDialog, i 0, i -1, i -1, i r6, i r7, i 0x0004)'

  ; the dialog is what shows through, so measure it rather than the parent
  System::Alloc 16
  Pop $1
  System::Call 'user32::GetClientRect(p $PageDialog, p r1)'
  System::Call '*$1(i.r2, i.r3, i.r4, i.r5)'
  System::Free $1
  ${If} $4 > 0
    StrCpy $PageW $4
  ${EndIf}
  ${If} $5 > 0
    StrCpy $PageH $5
  ${EndIf}

  IntOp $BtnY  $PageH - ${BTN_Y_OFF}
  IntOp $FootY $PageH - 46
  StrCpy $PriX 588
  StrCpy $SecX 452
FunctionEnd

; Full-bleed black background. Runs after images, before text and buttons.
; Full-bleed black background. Created before the text so it can be pushed to the
; bottom of the z-order; otherwise it covers the flat buttons and eats their clicks.
Function Backdrop
  IntOp $bk_w $PageW + 6
  IntOp $bk_h $PageH + 6
  ${NSD_CreateLabel} 0 0 $bk_w $bk_h ""
  Pop $0
  SetCtlColors $0 ${BG} ${BG}
FunctionEnd

; The mark is a bitmap, so it must exist before the black backdrop is painted.
Function RailBackground
  ${NSD_CreateBitmap} 0 0 250 560 ""
  Pop $0
  ${NSD_SetImage} $0 "$PLUGINSDIR\rail.bmp" $ImageHandle
  ; paint it first: everything else, opaque or not, must sit on top of it
  System::Call 'user32::SetWindowPos(p $0, p 1, i 0, i 0, i 0, i 0, i 0x0013)'
FunctionEnd

Function RailLogo
  Call RailBackground
FunctionEnd

; Divider and tagline, drawn on top of the backdrop.
Function RailText
  !insertmacro RAILRULE

  ${NSD_CreateLabel} 16 268 218 18 "Every game."
  Pop $0
  ${NSD_AddStyle} $0 ${SS_CENTER}
  SendMessage $0 0x0030 $FontTag 1
  SetCtlColors $0 ${FAINT} 0x0E0C16
  System::Call 'user32::GetWindowLongW(p $0, i -20) i.r1'
  IntOp $1 $1 & 0xFFFFFFDF
  System::Call 'user32::SetWindowLongW(p $0, i -20, i r1)'
  System::Call 'user32::SetWindowPos(p $0, p 0, i 0, i 0, i 0, i 0, i 0x0013)'

  ${NSD_CreateLabel} 16 288 218 18 "Its own place."
  Pop $0
  ${NSD_AddStyle} $0 ${SS_CENTER}
  SendMessage $0 0x0030 $FontTag 1
  SetCtlColors $0 ${FAINT} 0x0E0C16
  System::Call 'user32::GetWindowLongW(p $0, i -20) i.r1'
  IntOp $1 $1 & 0xFFFFFFDF
  System::Call 'user32::SetWindowLongW(p $0, i -20, i r1)'
  System::Call 'user32::SetWindowPos(p $0, p 0, i 0, i 0, i 0, i 0, i 0x0013)'
FunctionEnd

; --- Navigation ------------------------------------------------------------
Function GoNext
  GetDlgItem $0 $HWNDPARENT 1
  ${If} $0 != 0
    SendMessage $0 0x00F5 0 0
  ${EndIf}
FunctionEnd

Function GoBack
  GetDlgItem $0 $HWNDPARENT 2
  ${If} $0 != 0
    SendMessage $0 0x00F5 0 0
  ${EndIf}
FunctionEnd

Function GoCancel
  GetDlgItem $0 $HWNDPARENT 3
  ${If} $0 != 0
    SendMessage $0 0x00F5 0 0
  ${EndIf}
FunctionEnd

Function un.GoNext
  GetDlgItem $0 $HWNDPARENT 1
  ${If} $0 != 0
    SendMessage $0 0x00F5 0 0
  ${EndIf}
FunctionEnd

Function un.GoBack
  GetDlgItem $0 $HWNDPARENT 2
  ${If} $0 != 0
    SendMessage $0 0x00F5 0 0
  ${EndIf}
FunctionEnd

Function un.GoCancel
  GetDlgItem $0 $HWNDPARENT 3
  ${If} $0 != 0
    SendMessage $0 0x00F5 0 0
  ${EndIf}
FunctionEnd

; Push a percentage, then Call SetPct.
Function SetPct
  Exch $0
  IntOp $0 $0 * ${CW}
  IntOp $0 $0 / 100
  ${If} $0 < 4
    StrCpy $0 4
  ${EndIf}
  System::Call 'user32::SetWindowPos(p $hFill, i 0, i 0, i 0, i $0, i 3, i 0x0016)'
  Pop $0
FunctionEnd

; --- 1. Welcome ------------------------------------------------------------
Function WelcomePage
  Call PageFrame

  Call RailLogo

  Call Backdrop
  Call RailText

  !insertmacro HEAD "WELCOME" "Your games. One place." "A home for your entire PC game collection."

  !insertmacro BODY "Bring your library together, keep track of your adventures, and" ${CX} 186 ${CW}
  !insertmacro BODY "get straight into playing." ${CX} 206 ${CW}

  !insertmacro FOOT "WELCOME TO SILO   VERSION ${APP_VERSION}"

  !insertmacro FLATBTN $PriX $BtnY 124 ${BTN_H} "Get Started" ${ACCENT_INK} ${ACCENT} WelcomeNext

  nsDialogs::Show
FunctionEnd

Function WelcomeNext
  Call GoNext
FunctionEnd

; --- 2. License ------------------------------------------------------------
Function LicensePage
  Call PageFrame

  Call RailLogo

  ; multiline has to be created with the control: CHILD|VISIBLE|CLIPSIBLINGS|
  ; CLIPCHILDREN|GROUP|TABSTOP|BORDER|VSCROLL|ES_MULTILINE|ES_AUTOVSCROLL|ES_READONLY
  ; ES_MULTILINE only works when it is given at creation, so build the style bits
  ; here instead of relying on ${NSD_CreateText}
  ; nsDialogs cannot create a multiline edit, so create it directly on the page
  ; dialog: WS_CHILD|WS_VISIBLE|WS_BORDER|WS_VSCROLL|ES_AUTOVSCROLL|ES_MULTILINE|ES_READONLY
  System::Call 'user32::CreateWindowEx(i 0, w "EDIT", w "", i 0x50A10844,     i ${CX}, i 258, i ${CW}+16, i 132, p $PageDialog, p 0, p 0, p 0) i .r1'
  StrCpy $hLicense $1
  SendMessage $hLicense 0x0030 $FontBody 1
  SetCtlColors $hLicense ${BODY} ${PANEL}

  Call Backdrop
  Call RailText

  !insertmacro HEAD "LICENSE" "Built for your library." "Before we begin, take a moment to review the software license."

  !insertmacro BODY "Please read the following agreement carefully. You must accept its" ${CX} 186 ${CW}
  !insertmacro BODY "terms to continue with the installation." ${CX} 206 ${CW}

  !insertmacro MICRO "SOFTWARE LICENSE AGREEMENT" ${CX} 238 400

  Call LoadLicense

  !insertmacro FLATBTN 432 $BtnY 112 ${BTN_H} "Decline" ${TEXT_SUB} ${BTN_DARK} LicenseDecline
  !insertmacro FLATBTN 556 $BtnY 156 ${BTN_H} "Accept && Continue" ${ACCENT_INK} ${ACCENT} LicenseAccept

  nsDialogs::Show
FunctionEnd

Function LoadLicense
  FileOpen $9 "$PLUGINSDIR\LICENSE.txt" r
  ${If} $9 == ""
    ${NSD_SetText} $hLicense "License text not found."
    Return
  ${EndIf}

  licloop:
    FileRead $9 $1
    ${If} $1 == ""
      Goto licdone
    ${EndIf}
    StrCpy $1 "$1"   ; FileRead keeps the line ending
    SendMessage $hLicense 0x00C2 0 "STR:$1"
    Goto licloop

  licdone:
    FileClose $9
FunctionEnd

Function LicenseDecline
  Call GoCancel
FunctionEnd

Function LicenseAccept
  StrCpy $LicenseAccepted "1"
  Call GoNext
FunctionEnd

; --- 3. Location -----------------------------------------------------------
Function LocationPage
  Call PageFrame

  Call RailLogo

  ${NSD_CreateText} ${CX} 258 312 32 "$INSTDIR"
  Pop $hPathField
  SendMessage $hPathField 0x0030 $FontBody 1
  SetCtlColors $hPathField ${TEXT} ${PANEL}

  Call Backdrop
  Call RailText

  !insertmacro HEAD "LOCATION" "Choose your space." "Select where SILO will live on your computer."

  !insertmacro MICRO "INSTALLATION DIRECTORY" ${CX} 238 400

  !insertmacro FLATBTN 620 258 92 32 "Browse" ${TEXT_SUB} ${BTN_DARK} BrowseFolder

  !insertmacro BODY "You can change this location before continuing." ${CX} 302 ${CW}

  !insertmacro FLATBTN $SecX $BtnY 112 ${BTN_H} "Back" ${TEXT_SUB} ${BTN_DARK} LocationBack
  !insertmacro FLATBTN $PriX $BtnY 124 ${BTN_H} "Install" ${ACCENT_INK} ${ACCENT} LocationNext

  nsDialogs::Show
FunctionEnd

Function BrowseFolder
  ${NSD_GetText} $hPathField $0
  nsDialogs::SelectFolderDialog "Choose where SILO is installed" "$0"
  Pop $0
  ${If} $0 != error
    ${NSD_SetText} $hPathField "$0"
  ${EndIf}
FunctionEnd

Function LocationNext
  ${NSD_GetText} $hPathField $0
  ${If} $0 == ""
    MessageBox MB_OK|MB_ICONEXCLAMATION "Choose a folder to install SILO into."
    Return
  ${EndIf}
  StrCpy $INSTDIR $0
  Call GoNext
FunctionEnd

Function LocationBack
  Call GoBack
FunctionEnd

; --- 4. Progress -----------------------------------------------------------
Function ProgressPage
  Call PageFrame

  Call RailLogo

  Call Backdrop
  Call RailText

  !insertmacro HEAD "INSTALLING" "Making room for your games." "SILO is finding its place on your system."

  !insertmacro BODY "Installing application files..." ${CX} 232 ${CW}

  ${NSD_CreateLabel} ${CX} 262 ${CW} 3 ""
  Pop $0
  SetCtlColors $0 ${HAIRLINE} ${HAIRLINE}

  ${NSD_CreateLabel} ${CX} 262 4 3 ""
  Pop $hFill
  SetCtlColors $hFill ${ACCENT} ${ACCENT}

  !insertmacro FOOT "PLEASE KEEP THIS WINDOW OPEN"

  StrCpy $StepVar "0"
  ${NSD_CreateTimer} ProgressTick 90

  nsDialogs::Show
FunctionEnd

Function ProgressTick
  ${If} $StepVar == "0"
    StrCpy $StepVar "1"
    CreateDirectory "$INSTDIR"
    Push 10
    Call SetPct
    Return
  ${EndIf}

  ${If} $StepVar == "1"
    StrCpy $StepVar "2"
    SetOutPath "$INSTDIR"
    File "/oname=${APP_EXE}" "${SRC_EXE}"
    Push 55
    Call SetPct
    Return
  ${EndIf}

  ${If} $StepVar == "2"
    StrCpy $StepVar "3"
    SetOutPath "$INSTDIR"
    File "/oname=icon.ico" "${SRC_ICO}"
    WriteUninstaller "$INSTDIR\uninstall.exe"
    Push 78
    Call SetPct
    Return
  ${EndIf}

  ${If} $StepVar == "3"
    StrCpy $StepVar "4"
    CreateDirectory "$SMPROGRAMS\${APP_NAME}"
    CreateShortCut "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}" 0

    WriteRegStr HKCU "Software\${APP_ID}" "InstallDir" "$INSTDIR"

    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}" "DisplayName"     "${APP_NAME}"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}" "DisplayVersion"  "${APP_VERSION}"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}" "DisplayIcon"     "$INSTDIR\${APP_EXE}"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}" "UninstallString" '"$INSTDIR\uninstall.exe"'
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}" "Publisher"       "Anton John"
    WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}" "NoModify" 1
    WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}" "NoRepair" 1

    ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
    WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}" "EstimatedSize" $0

    Push 100
    Call SetPct
    Return
  ${EndIf}

  ${If} $StepVar == "4"
    StrCpy $StepVar "5"
    Call GoNext
    Return
  ${EndIf}
FunctionEnd

; --- 5. Complete -----------------------------------------------------------
Function FinishPage
  Call PageFrame

  ${NSD_CreateBitmap} 0 0 18 18 ""
  Pop $CheckBox
  ${NSD_AddStyle} $CheckBox ${SS_NOTIFY}
  ${If} $LaunchOn == "1"
    ${NSD_SetImage} $CheckBox "$PLUGINSDIR\check_on.bmp" $ToggleImage
  ${Else}
    ${NSD_SetImage} $CheckBox "$PLUGINSDIR\check_off.bmp" $ToggleImage
  ${EndIf}
  ${NSD_OnClick} $CheckBox ToggleLaunch

  Call RailLogo

  Call Backdrop
  Call RailText

  !insertmacro HEAD "COMPLETE" "You're all set." "Your library has a new home."

  !insertmacro BODY "SILO is installed and ready. Open it, bring your games together, and" ${CX} 186 ${CW}
  !insertmacro BODY "make the space your own." ${CX} 206 ${CW}

  ; the toggle bitmap is created before the backdrop, so place it afterwards
  System::Call 'user32::SetWindowPos(p $CheckBox, i 0, i ${CX}, i 288, i 18, i 18, i 0x0014)' 

  ${NSD_CreateLabel} 326 288 260 18 "Launch SILO"
  Pop $0
  SendMessage $0 0x0030 $FontBody 1
  SetCtlColors $0 ${TEXT} ${BG}

  !insertmacro FOOT "THANK YOU FOR CHOOSING SILO"

  !insertmacro FLATBTN $PriX $BtnY 124 ${BTN_H} "Finish" ${ACCENT_INK} ${ACCENT} FinishClose

  nsDialogs::Show
FunctionEnd

Function ToggleLaunch
  ${If} $LaunchOn == "1"
    StrCpy $LaunchOn "0"
    ${NSD_SetImage} $CheckBox "$PLUGINSDIR\check_off.bmp" $ToggleImage
  ${Else}
    StrCpy $LaunchOn "1"
    ${NSD_SetImage} $CheckBox "$PLUGINSDIR\check_on.bmp" $ToggleImage
  ${EndIf}
FunctionEnd

Function FinishClose
  ${If} $LaunchOn == "1"
    ${If} ${FileExists} "$INSTDIR\${APP_EXE}"
      Exec '"$INSTDIR\${APP_EXE}"'
    ${EndIf}
  ${EndIf}
  Call GoNext
FunctionEnd

; --- Sections --------------------------------------------------------------
; All work happens on the page timers. These sections exist for NSIS
; bookkeeping and must never delete user data.
Section "Install"
SectionEnd

Section "Uninstall"
SectionEnd

; --- Uninstaller: intro ----------------------------------------------------
; Mockup page 6: "Leaving already?" with Keep SILO / Continue.
Function un.IntroPage
  Call un.PageFrame

  Call un.RailLogo

  Call un.Backdrop

  !insertmacro HEAD "UNINSTALL" "Leaving already?" "This will remove SILO from your computer."

  !insertmacro BODY "Continue only if you're sure you want to uninstall the" ${CX} 186 ${CW}
  !insertmacro BODY "application." ${CX} 206 ${CW}

  !insertmacro FOOT "SILO   VERSION ${APP_VERSION}"

  !insertmacro FLATBTN $SecX $BtnY 112 ${BTN_H} "Keep SILO" ${TEXT_SUB} ${BTN_DARK} un.CancelUninstall
  !insertmacro FLATBTN $PriX $BtnY 124 ${BTN_H} "Continue" ${ACCENT_INK} ${ACCENT} un.IntroNext

  nsDialogs::Show
FunctionEnd

Function un.IntroNext
  Call un.GoNext
FunctionEnd

Function un.CancelUninstall
  Call un.GoCancel
FunctionEnd

; --- Uninstaller: confirm --------------------------------------------------
; Mockup page 7: "One last thing." with Go Back / Uninstall.
Function un.ConfirmPage
  Call un.PageFrame

  Call un.RailLogo

  Call un.Backdrop

  !insertmacro HEAD "UNINSTALL" "One last thing." "Make sure you're ready to remove SILO."

  !insertmacro BODY "Your library, artwork, saves, backups and settings are kept." ${CX} 186 ${CW}
  !insertmacro BODY "Only the application itself is removed." ${CX} 206 ${CW}

  !insertmacro FOOT "YOUR LIBRARY AND SAVES ARE NOT TOUCHED"

  !insertmacro FLATBTN $SecX $BtnY 112 ${BTN_H} "Go Back" ${TEXT_SUB} ${BTN_DARK} un.ConfirmBack
  !insertmacro FLATBTN $PriX $BtnY 124 ${BTN_H} "Uninstall" ${ACCENT_INK} ${ACCENT} un.ConfirmNext

  nsDialogs::Show
FunctionEnd

Function un.ConfirmBack
  Call un.GoBack
FunctionEnd

Function un.ConfirmNext
  Call un.GoNext
FunctionEnd

; --- Uninstaller: work -----------------------------------------------------
; Mockup page 8: "Removing SILO." with a progress bar.
Function un.ProgressPage
  Call un.PageFrame

  Call un.RailLogo

  Call un.Backdrop

  !insertmacro HEAD "UNINSTALLING" "Removing SILO." "Removing application components from your system…"

  !insertmacro BODY "Removing application files..." ${CX} 232 ${CW}

  ${NSD_CreateLabel} ${CX} 262 ${CW} 3 ""
  Pop $0
  SetCtlColors $0 ${HAIRLINE} ${HAIRLINE}

  ${NSD_CreateLabel} ${CX} 262 4 3 ""
  Pop $hFill
  SetCtlColors $hFill ${ACCENT} ${ACCENT}

  !insertmacro FOOT "YOUR LIBRARY AND SAVES ARE NOT TOUCHED"

  StrCpy $StepVar "0"
  ${NSD_CreateTimer} un.ProgressTick 80

  nsDialogs::Show
FunctionEnd

Function un.ProgressTick
  ${If} $StepVar == "0"
    StrCpy $StepVar "1"
    Delete "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk"
    RMDir "$SMPROGRAMS\${APP_NAME}"
    Push 25
    Call un.SetPct
    Return
  ${EndIf}

  ${If} $StepVar == "1"
    StrCpy $StepVar "2"
    Delete "$INSTDIR\${APP_EXE}"
    Delete "$INSTDIR\icon.ico"
    Push 55
    Call un.SetPct
    Return
  ${EndIf}

  ${If} $StepVar == "2"
    StrCpy $StepVar "3"
    DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}"
    DeleteRegKey HKCU "Software\${APP_ID}"
    ; Deliberately does NOT touch %APPDATA%\com.silo.launcher or
    ; %LOCALAPPDATA%\com.silo.launcher: the user's library, artwork, saves,
    ; backups and settings are kept.
    Push 85
    Call un.SetPct
    Return
  ${EndIf}

  ${If} $StepVar == "3"
    StrCpy $StepVar "4"
    Delete /REBOOTOK "$INSTDIR\uninstall.exe"
    RMDir "$INSTDIR"
    Push 100
    Call un.SetPct
    Return
  ${EndIf}

  ${If} $StepVar == "4"
    StrCpy $StepVar "5"
    Call un.GoNext
    Return
  ${EndIf}
FunctionEnd

; --- Uninstaller: done -----------------------------------------------------
; Mockup page 9: "Until next time." with Close.
Function un.DonePage
  Call un.PageFrame

  Call un.RailLogo

  Call un.Backdrop

  !insertmacro HEAD "UNINSTALLED" "Until next time." "SILO has been uninstalled."

  !insertmacro BODY "Thanks for giving your game library a home with us." ${CX} 186 ${CW}
  !insertmacro BODY "Your library data was kept and can be picked up on reinstall." ${CX} 206 ${CW}

  !insertmacro FOOT "SILO   VERSION ${APP_VERSION}"

  !insertmacro FLATBTN $PriX $BtnY 124 ${BTN_H} "Close" ${ACCENT_INK} ${ACCENT} un.DoneClose

  nsDialogs::Show
FunctionEnd

Function un.DoneClose
  Call un.GoNext
FunctionEnd

; --- Uninstaller frame -----------------------------------------------------
Function un.PageFrame
  nsDialogs::Create 1018
  Pop $PageDialog
  ${If} $PageDialog != error
    System::Call 'gdi32::CreateSolidBrush(i 0) p.r8'
    ${If} $8 != 0
      System::Call 'user32::SetClassLongPtrW(p $PageDialog, i -10, p r8)'
    ${EndIf}
  ${EndIf}
  ${If} $PageDialog == error
    Abort
  ${EndIf}

  System::Call 'user32::GetSystemMetrics(i 0) i.r0'
  System::Call 'user32::GetSystemMetrics(i 1) i.r1'
  IntOp $2 $0 - ${PAGE_W}
  IntOp $3 $1 - ${PAGE_H}
  IntOp $2 $2 / 2
  IntOp $3 $3 / 2
  System::Call 'user32::SetWindowPos(i $HWNDPARENT, i 0, i $2, i $3, i ${PAGE_W}, i ${PAGE_H}, i 0x0004)'

  ; the frame eats a few pixels, so grow the window until the client area is
  ; exactly PAGE_W x PAGE_H. Otherwise the page dialog leaves a light edge.
  System::Alloc 16
  Pop $1
  System::Call 'user32::GetClientRect(p $HWNDPARENT, p r1)'
  System::Call '*$1(i.r2, i.r3, i.r4, i.r5)'
  System::Free $1
  IntOp $0 ${PAGE_W} - $4
  IntOp $2 ${PAGE_W} + $0
  IntOp $0 ${PAGE_H} - $5
  IntOp $3 ${PAGE_H} + $0
  System::Call 'user32::GetSystemMetrics(i 0) i.r0'
  IntOp $0 $0 + 0
  IntOp $5 $2 / 1
  IntOp $6 ${PAGE_W} / 1
  System::Call 'user32::GetSystemMetrics(i 0) i.r0'
  System::Call 'user32::GetSystemMetrics(i 1) i.r1'
  IntOp $4 $0 - $2
  IntOp $4 $4 / 2
  IntOp $5 $1 - $3
  IntOp $5 $5 / 2
  System::Call 'user32::SetWindowPos(i $HWNDPARENT, i 0, i $4, i $5, i $2, i $3, i 0x0004)'

  GetDlgItem $9 $HWNDPARENT 1
  System::Call 'user32::ShowWindow(p r9, i 0)'
  GetDlgItem $9 $HWNDPARENT 2
  System::Call 'user32::ShowWindow(p r9, i 0)'
  GetDlgItem $9 $HWNDPARENT 3
  System::Call 'user32::ShowWindow(p r9, i 0)'

  System::Call 'dwmapi::DwmSetWindowAttribute(p $HWNDPARENT, i 20, *i 1, i 4)'
  System::Call 'dwmapi::DwmSetWindowAttribute(p $HWNDPARENT, i 19, *i 1, i 4)'
  System::Call 'dwmapi::DwmSetWindowAttribute(p $HWNDPARENT, i 34, *i 0, i 4)'
  System::Call 'dwmapi::DwmSetWindowAttribute(p $HWNDPARENT, i 35, *i 0, i 4)'
  System::Call 'user32::SetForegroundWindow(p $HWNDPARENT)'
  System::Call 'user32::SetActiveWindow(p $HWNDPARENT)'

  System::Call 'gdi32::CreateSolidBrush(i 0x000000) p.r8'
  ${If} $8 != 0
    System::Call 'user32::SetClassLongPtrW(p $HWNDPARENT, i -10, p r8)'
  ${EndIf}

  System::Alloc 16
  Pop $1
  System::Call 'user32::GetClientRect(p $HWNDPARENT, p r1)'
  System::Call '*$1(i.r2, i.r3, i.r4, i.r5)'
  System::Free $1
  StrCpy $PageW $4
  StrCpy $PageH $5

  IntOp $6 $4 + 2
  IntOp $7 $5 + 2
  System::Call 'user32::SetWindowPos(p $PageDialog, i 0, i -1, i -1, i r6, i r7, i 0x0004)'

  ; the dialog is what shows through, so measure it rather than the parent
  System::Alloc 16
  Pop $1
  System::Call 'user32::GetClientRect(p $PageDialog, p r1)'
  System::Call '*$1(i.r2, i.r3, i.r4, i.r5)'
  System::Free $1
  ${If} $4 > 0
    StrCpy $PageW $4
  ${EndIf}
  ${If} $5 > 0
    StrCpy $PageH $5
  ${EndIf}

  IntOp $BtnY  $PageH - ${BTN_Y_OFF}
  IntOp $FootY $PageH - 46
  StrCpy $PriX 588
  StrCpy $SecX 452
FunctionEnd

Function un.Backdrop
  IntOp $bk_w $PageW + 6
  IntOp $bk_h $PageH + 6
  ${NSD_CreateLabel} 0 0 $bk_w $bk_h ""
  Pop $0
  SetCtlColors $0 ${BG} ${BG}
FunctionEnd

Function un.RailBackground
  ${NSD_CreateBitmap} 0 0 250 520 ""
  Pop $0
  ${NSD_SetImage} $0 "$PLUGINSDIR\unrail.bmp" $ImageHandle
  System::Call 'user32::SetWindowPos(p $0, p 1, i 0, i 0, i 0, i 0, i 0x0013)'
FunctionEnd

Function un.RailLogo
  Call un.RailBackground
FunctionEnd

Function un.SetPct
  Exch $0
  IntOp $0 $0 * ${CW}
  IntOp $0 $0 / 100
  ${If} $0 < 4
    StrCpy $0 4
  ${EndIf}
  System::Call 'user32::SetWindowPos(p $hFill, i 0, i 0, i 0, i $0, i 3, i 0x0016)'
  Pop $0
FunctionEnd

Function un.onInit
  InitPluginsDir
  File /oname=$PLUGINSDIR\unrail.bmp "gen\uninstall_rail.bmp"

  !insertmacro MKFONT $FontH1    32 400
  !insertmacro MKFONT $FontSub   14 400
  !insertmacro MKFONT $FontBody  13 400
  !insertmacro MKFONT $FontMicro 10 600
  !insertmacro MKFONT $FontBtn   13 600
  !insertmacro MKFONT $FontTag   12 400
FunctionEnd
