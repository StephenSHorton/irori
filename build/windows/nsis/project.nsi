Unicode true

####
## Please note: Template replacements don't work in this file. They are provided with default defines like
## mentioned underneath.
## If the keyword is not defined, "wails_tools.nsh" will populate them.
## If they are defined here, "wails_tools.nsh" will not touch them. This allows you to use this project.nsi manually
## from outside of Wails for debugging and development of the installer.
## 
## For development first make a wails nsis build to populate the "wails_tools.nsh":
## > wails build --target windows/amd64 --nsis
## Then you can call makensis on this file with specifying the path to your binary:
## For a AMD64 only installer:
## > makensis -DARG_WAILS_AMD64_BINARY=..\..\bin\app.exe
## For a ARM64 only installer:
## > makensis -DARG_WAILS_ARM64_BINARY=..\..\bin\app.exe
## For a installer with both architectures:
## > makensis -DARG_WAILS_AMD64_BINARY=..\..\bin\app-amd64.exe -DARG_WAILS_ARM64_BINARY=..\..\bin\app-arm64.exe
####
## The following information is taken from the wails_tools.nsh file, but they can be overwritten here.
####
## !define INFO_PROJECTNAME    "my-project" # Default "irori"
## !define INFO_COMPANYNAME    "My Company" # Default "Stephen Horton"
## !define INFO_PRODUCTNAME    "My Product Name" # Default "Irori"
## !define INFO_PRODUCTVERSION "1.0.0"     # Default "0.1.0"
## !define INFO_COPYRIGHT      "(c) Now, My Company" # Default "© 2026, My Company"
###
## !define PRODUCT_EXECUTABLE  "Application.exe"      # Default "${INFO_PROJECTNAME}.exe"
## !define UNINST_KEY_NAME     "UninstKeyInRegistry"  # Default "${INFO_COMPANYNAME}${INFO_PRODUCTNAME}"
####
## !define REQUEST_EXECUTION_LEVEL "admin"            # Default "admin"  see also https://nsis.sourceforge.io/Docs/Chapter4.html
## !define WAILS_INSTALL_SCOPE     "user"             # Default "machine" - set to "user" for per-user install ($LOCALAPPDATA) without UAC prompt
####
## Irori installs per user (%LOCALAPPDATA%\Programs\Irori, no UAC prompt) so the
## running app can launch its own silent update installer. Pass
## -DWAILS_INSTALL_SCOPE=machine to override.
####
!ifndef WAILS_INSTALL_SCOPE
    !define WAILS_INSTALL_SCOPE "user"
!endif
####
## Include the wails tools
####
!include "wails_tools.nsh"

# The version information for this two must consist of 4 parts
VIProductVersion "${INFO_PRODUCTVERSION}.0"
VIFileVersion    "${INFO_PRODUCTVERSION}.0"

VIAddVersionKey "CompanyName"     "${INFO_COMPANYNAME}"
VIAddVersionKey "FileDescription" "${INFO_PRODUCTNAME} Installer"
VIAddVersionKey "ProductVersion"  "${INFO_PRODUCTVERSION}"
VIAddVersionKey "FileVersion"     "${INFO_PRODUCTVERSION}"
VIAddVersionKey "LegalCopyright"  "${INFO_COPYRIGHT}"
VIAddVersionKey "ProductName"     "${INFO_PRODUCTNAME}"

# Enable HiDPI support. https://nsis.sourceforge.io/Reference/ManifestDPIAware
ManifestDPIAware true

!include "MUI.nsh"

!define MUI_ICON "..\icon.ico"
!define MUI_UNICON "..\icon.ico"
# !define MUI_WELCOMEFINISHPAGE_BITMAP "resources\leftimage.bmp" #Include this to add a bitmap on the left side of the Welcome Page. Must be a size of 164x314
!define MUI_FINISHPAGE_NOAUTOCLOSE # Wait on the INSTFILES page so the user can take a look into the details of the installation steps
!define MUI_ABORTWARNING # This will warn the user if they exit from the installer.

# Offer to launch Irori when an INTERACTIVE install finishes (checked by
# default). The per-user installer is not elevated, so Irori runs at the normal
# user level. Silent (auto-update) installs skip the Finish page and relaunch
# from the end of the install Section instead.
!define MUI_FINISHPAGE_RUN "$INSTDIR\${PRODUCT_EXECUTABLE}"
!define MUI_FINISHPAGE_RUN_TEXT "Launch ${INFO_PRODUCTNAME}"

!insertmacro MUI_PAGE_WELCOME # Welcome to the installer page.
# !insertmacro MUI_PAGE_LICENSE "resources\eula.txt" # Adds a EULA page to the installer
!insertmacro MUI_PAGE_DIRECTORY # In which folder install page.
!insertmacro MUI_PAGE_INSTFILES # Installing page.
!insertmacro MUI_PAGE_FINISH # Finished installation page.

!insertmacro MUI_UNPAGE_INSTFILES # Uninstalling page

!insertmacro MUI_LANGUAGE "English" # Set the Language of the installer

## The following two statements can be used to sign the installer and the uninstaller. The path to the binaries are provided in %1
#!uninstfinalize 'signtool --file "%1"'
#!finalize 'signtool --file "%1"'

Name "${INFO_PRODUCTNAME}"
OutFile "..\..\..\bin\${INFO_PROJECTNAME}-${ARCH}-installer.exe" # Name of the installer's file.
!if "${WAILS_INSTALL_SCOPE}" == "user"
    InstallDir "$LOCALAPPDATA\Programs\${INFO_PRODUCTNAME}"
!else
    InstallDir "$PROGRAMFILES64\${INFO_COMPANYNAME}\${INFO_PRODUCTNAME}"
!endif
ShowInstDetails show # This will always show the installation details.

# "1" on a first install, "0" on an update or reinstall (irori.exe already
# present). Gates the Desktop shortcut so an update never brings back a
# shortcut the user deleted.
Var FreshInstall

Function .onInit
   !insertmacro wails.checkArchitecture
FunctionEnd

Section
    !insertmacro wails.setShellContext

    !insertmacro wails.webview2runtime

    # AUTO-UPDATE: a SILENT install is launched by the running Irori, which then
    # quits to release its lock on irori.exe. Wait for that exit so wails.files
    # can overwrite the exe and the old instance's single-instance lock is gone
    # before the relaunch below. An interactive install falls straight through.
    #
    # The probe opens the exe for writing (append, no truncation), which is what
    # File will do. It must NOT be a rename: Windows lets a running exe be
    # renamed, so a rename probe passes while the old process is still shutting
    # down, File then fails (silent mode ignores that), and the relaunch hands
    # off to the dying instance. Toru shipped that bug once.
    #
    # Allow up to 60 s. If the exe is still locked after that, abort and change
    # nothing: the running Irori offers the update again at its next start.
    ${If} ${Silent}
        StrCpy $0 0
        irori_wait_unlock:
            IfFileExists "$INSTDIR\${PRODUCT_EXECUTABLE}" 0 irori_unlocked
            ClearErrors
            FileOpen $1 "$INSTDIR\${PRODUCT_EXECUTABLE}" a
            IfErrors irori_still_locked
            FileClose $1
            Goto irori_unlocked
            irori_still_locked:
            IntOp $0 $0 + 1
            IntCmp $0 300 irori_gave_up 0 irori_gave_up
            Sleep 200
            Goto irori_wait_unlock
        irori_gave_up:
            SetErrorLevel 3
            Abort "irori.exe is still in use; update skipped"
        irori_unlocked:
    ${EndIf}

    # Fresh install or update? Decided before any files land.
    StrCpy $FreshInstall "1"
    ${If} ${FileExists} "$INSTDIR\${PRODUCT_EXECUTABLE}"
        StrCpy $FreshInstall "0"
    ${EndIf}

    SetOutPath $INSTDIR

    !insertmacro wails.files

    # The Start-menu shortcut is always (re)created; the Desktop one only on a
    # fresh install.
    CreateShortcut "$SMPROGRAMS\${INFO_PRODUCTNAME}.lnk" "$INSTDIR\${PRODUCT_EXECUTABLE}"
    ${If} $FreshInstall == "1"
        CreateShortCut "$DESKTOP\${INFO_PRODUCTNAME}.lnk" "$INSTDIR\${PRODUCT_EXECUTABLE}"
    ${EndIf}

    !insertmacro wails.associateFiles
    !insertmacro wails.associateCustomProtocols

    !insertmacro wails.writeUninstaller

    # AUTO-UPDATE: after a silent update, relaunch the new Irori UNLESS the
    # updater passed /NORELAUNCH. The running Irori launches us two ways
    # (internal/update):
    #   "Restart now"           -> installer /S              (relaunch)
    #   quit with update ready  -> installer /S /NORELAUNCH  (stay closed)
    # ${GetParameters}/${GetOptions} come from FileFunc.nsh (wails_tools.nsh).
    ${If} ${Silent}
        ${GetParameters} $R0
        ClearErrors
        ${GetOptions} $R0 "/NORELAUNCH" $R1
        ${If} ${Errors}
            Exec '"$INSTDIR\${PRODUCT_EXECUTABLE}"'
        ${EndIf}
    ${EndIf}
SectionEnd

Section "uninstall"
    !insertmacro wails.setShellContext

    RMDir /r "$AppData\${PRODUCT_EXECUTABLE}" # Remove the WebView2 DataPath

    RMDir /r $INSTDIR

    Delete "$SMPROGRAMS\${INFO_PRODUCTNAME}.lnk"
    Delete "$DESKTOP\${INFO_PRODUCTNAME}.lnk"

    # Launch at login (settings_service.go autostartID) must not outlive the app.
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Irori"

    !insertmacro wails.unassociateFiles
    !insertmacro wails.unassociateCustomProtocols

    !insertmacro wails.deleteUninstaller
SectionEnd
