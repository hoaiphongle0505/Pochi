Unicode true
!define APP "Pochi"
!ifndef VER
  !define VER "3.4.0"
!endif
!ifndef EXE
  !define EXE "target\release\pochi.exe"
!endif
Name "${APP}"
OutFile "dist\Pochi-Setup-${VER}.exe"
InstallDir "$PROGRAMFILES\Pochi Native"
RequestExecutionLevel admin
SetCompressor /SOLID lzma
Icon "res\icon.ico"
UninstallIcon "res\icon.ico"
VIProductVersion "${VER}.0"
VIAddVersionKey "ProductName" "Pochi"
VIAddVersionKey "FileDescription" "Pochi notch for Windows"
VIAddVersionKey "FileVersion" "${VER}"
VIAddVersionKey "LegalCopyright" "Pochi"

!include "MUI2.nsh"
!define MUI_ICON "res\icon.ico"
!define MUI_UNICON "res\icon.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\pochi.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start Pochi"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Section "Install"
  SetShellVarContext all
  ; stop any running Pochi (old Electron build or a previous native build)
  nsExec::Exec 'taskkill /F /IM pochi.exe'
  Sleep 400
  ; the old Electron build registered its own auto-start entries
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "electron.app.Pochi"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "com.pochi.app"
  SetOutPath "$INSTDIR"
  File "/oname=pochi.exe" "${EXE}"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortCut "$SMPROGRAMS\Pochi.lnk" "$INSTDIR\pochi.exe"
  CreateShortCut "$DESKTOP\Pochi.lnk" "$INSTDIR\pochi.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Pochi" '"$INSTDIR\pochi.exe"'
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PochiNative" "DisplayName" "Pochi"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PochiNative" "DisplayVersion" "${VER}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PochiNative" "Publisher" "Pochi"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PochiNative" "DisplayIcon" "$INSTDIR\pochi.exe"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PochiNative" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PochiNative" "NoModify" 1
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PochiNative" "NoRepair" 1
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PochiNative" "EstimatedSize" 2000
SectionEnd

Section "Uninstall"
  SetShellVarContext all
  nsExec::Exec 'taskkill /F /IM pochi.exe'
  Sleep 300
  Delete "$INSTDIR\pochi.exe"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Pochi.lnk"
  Delete "$DESKTOP\Pochi.lnk"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Pochi"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PochiNative"
SectionEnd
