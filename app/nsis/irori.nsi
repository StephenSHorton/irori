Unicode true

####
## Irori's per-user installer: a port of the Go build's
## build/windows/nsis/project.nsi + wails_tools.nsh (Go be67a86), with the
## Wails plumbing replaced by plain defines. It keeps everything the Go
## updater (internal/update) and the existing install rely on
## (surface.md 7.1, architecture.md 7.2):
##   - per-user %LOCALAPPDATA%\Programs\Irori\irori.exe, no UAC prompt;
##   - the uninstall key "Stephen HortonIrori" (so Settings > Apps keeps one
##     Irori entry across the Go to Rust update);
##   - /S, /NORELAUNCH, /WAITPID=<pid> with a 60 s wait, the append-open
##     probe, irori.exe.old[N] rename-aside, exit code 3 and the old Irori
##     relaunched when it gives up, the new Irori relaunched after a silent
##     install unless /NORELAUNCH;
##   - the Start menu shortcut always, the Desktop one on a fresh install;
##   - the uninstaller removes Run\Irori and both WebView2 data folders
##     (Wails' %APPDATA%\irori.exe and Tauri's
##     %LOCALAPPDATA%\com.stephenshorton.irori).
##
## Built by scripts/release.ps1 with makensis directly, not Tauri's bundler
## (whose template differs in all of the above):
##
##   makensis -DINFO_PRODUCTVERSION=0.6.0 -DIRORI_BINARY=<irori.exe>
##            -DIRORI_OUTFILE=<installer.exe>
##            [-DIRORI_WEBVIEW2_BOOTSTRAPPER=<MicrosoftEdgeWebview2Setup.exe>]
##            app\nsis\irori.nsi
##
## Test-only defines (scripts/installer-check.ps1):
##   IRORI_SCRATCH        touch nothing outside $INSTDIR: no shortcuts, no
##                        registry, no WebView2 step, no Run or WebView2
##                        removal on uninstall. Install with /D=<scratch dir>.
##   IRORI_SCRATCH_INSTDIR  with IRORI_SCRATCH: the default $INSTDIR, so an
##                        install without /D= (the Go updater's) stays in a
##                        scratch dir (tools/rehearse).
##   IRORI_WAITPID_MS     the /WAITPID wait (default 60000).
####

!include "LogicLib.nsh"
!include "x64.nsh"
!include "WinVer.nsh"
!include "FileFunc.nsh"

!ifndef INFO_PROJECTNAME
    !define INFO_PROJECTNAME "irori"
!endif
!ifndef INFO_COMPANYNAME
    !define INFO_COMPANYNAME "Stephen Horton"
!endif
!ifndef INFO_PRODUCTNAME
    !define INFO_PRODUCTNAME "Irori"
!endif
!ifndef INFO_PRODUCTVERSION
    !error "pass -DINFO_PRODUCTVERSION=X.Y.Z"
!endif
!ifndef INFO_COPYRIGHT
    !define INFO_COPYRIGHT "(c) 2026, Stephen Horton"
!endif
!ifndef IRORI_BINARY
    !error "pass -DIRORI_BINARY=<path to irori.exe>"
!endif
!ifndef IRORI_OUTFILE
    !error "pass -DIRORI_OUTFILE=<installer path>"
!endif
!ifndef IRORI_WAITPID_MS
    !define IRORI_WAITPID_MS 60000
!endif
!define PRODUCT_EXECUTABLE "${INFO_PROJECTNAME}.exe"
!ifdef IRORI_SCRATCH
    !define UNINST_KEY_NAME "${INFO_COMPANYNAME}${INFO_PRODUCTNAME}-scratch"
!else
    !define UNINST_KEY_NAME "${INFO_COMPANYNAME}${INFO_PRODUCTNAME}"
!endif
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${UNINST_KEY_NAME}"
# The Tauri identifier (app/tauri.conf.json): names its WebView2 data folder.
!define TAURI_IDENTIFIER "com.stephenshorton.irori"

RequestExecutionLevel user

# The version information must have four parts.
VIProductVersion "${INFO_PRODUCTVERSION}.0"
VIFileVersion    "${INFO_PRODUCTVERSION}.0"

VIAddVersionKey "CompanyName"     "${INFO_COMPANYNAME}"
VIAddVersionKey "FileDescription" "${INFO_PRODUCTNAME} Installer"
VIAddVersionKey "ProductVersion"  "${INFO_PRODUCTVERSION}"
VIAddVersionKey "FileVersion"     "${INFO_PRODUCTVERSION}"
VIAddVersionKey "LegalCopyright"  "${INFO_COPYRIGHT}"
VIAddVersionKey "ProductName"     "${INFO_PRODUCTNAME}"

ManifestDPIAware true

!include "MUI.nsh"

!define MUI_ICON "..\icons\icon.ico"
!define MUI_UNICON "..\icons\icon.ico"
!define MUI_FINISHPAGE_NOAUTOCLOSE
!define MUI_ABORTWARNING

# An interactive install offers to launch Irori at the end; a silent
# (auto-update) install relaunches from the end of the install Section.
!define MUI_FINISHPAGE_RUN "$INSTDIR\${PRODUCT_EXECUTABLE}"
!define MUI_FINISHPAGE_RUN_TEXT "Launch ${INFO_PRODUCTNAME}"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Name "${INFO_PRODUCTNAME}"
OutFile "${IRORI_OUTFILE}"
!ifdef IRORI_SCRATCH_INSTDIR
    # A scratch install the Go updater can run as is (it passes no /D=).
    !ifndef IRORI_SCRATCH
        !error "IRORI_SCRATCH_INSTDIR needs IRORI_SCRATCH"
    !endif
    InstallDir "${IRORI_SCRATCH_INSTDIR}"
!else
    InstallDir "$LOCALAPPDATA\Programs\${INFO_PRODUCTNAME}"
!endif
ShowInstDetails show

# "1" on a first install, "0" on an update or reinstall (irori.exe already
# present). Gates the Desktop shortcut so an update never brings back a
# shortcut the person deleted.
Var FreshInstall

# Starts $INSTDIR\irori.exe unless the command line has /NORELAUNCH. Used after
# a silent update and on the silent gave-up path (then it starts the old exe).
Function IroriRelaunchUnlessNoRelaunch
    ${GetParameters} $R0
    ClearErrors
    ${GetOptions} $R0 "/NORELAUNCH" $R1
    ${If} ${Errors}
        Exec '"$INSTDIR\${PRODUCT_EXECUTABLE}"'
    ${EndIf}
FunctionEnd

# wails.checkArchitecture: Windows 10+ on amd64, else exit 64 / 65 silently.
Function .onInit
    ${IfNot} ${AtLeastWin10}
        ${If} ${Silent}
            SetErrorLevel 64
            Abort
        ${EndIf}
        MessageBox MB_OK "This product is only supported on Windows 10 (Server 2016) and later."
        Quit
    ${EndIf}
    ${IfNot} ${IsNativeAMD64}
        ${If} ${Silent}
            SetErrorLevel 65
            Abort
        ${EndIf}
        MessageBox MB_OK "This product can't be installed on the current Windows architecture. Supports: amd64"
        Quit
    ${EndIf}
FunctionEnd

!macro IroriWebView2
    !ifndef IRORI_SCRATCH
    !ifdef IRORI_WEBVIEW2_BOOTSTRAPPER
        SetRegView 64
        # Installed machine-wide, or for this user: nothing to do.
        ReadRegStr $0 HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
        ${If} $0 == ""
            ReadRegStr $0 HKCU "Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
        ${EndIf}
        ${If} $0 == ""
            SetDetailsPrint both
            DetailPrint "Installing: WebView2 Runtime"
            SetDetailsPrint listonly
            InitPluginsDir
            CreateDirectory "$pluginsdir\webview2bootstrapper"
            SetOutPath "$pluginsdir\webview2bootstrapper"
            File "/oname=MicrosoftEdgeWebview2Setup.exe" "${IRORI_WEBVIEW2_BOOTSTRAPPER}"
            ExecWait '"$pluginsdir\webview2bootstrapper\MicrosoftEdgeWebview2Setup.exe" /silent /install'
            SetDetailsPrint both
        ${EndIf}
    !endif
    !endif
!macroend

Section
    SetShellVarContext current

    !insertmacro IroriWebView2

    # Fresh install or update? Decided before anything touches irori.exe: the
    # swap below can rename it aside (an `irori mcp` still holds it), and an
    # update must not then look fresh and bring back a Desktop shortcut the
    # person deleted. (Go's project.nsi decided after the swap.)
    StrCpy $FreshInstall "1"
    ${If} ${FileExists} "$INSTDIR\${PRODUCT_EXECUTABLE}"
        StrCpy $FreshInstall "0"
    ${EndIf}

    # AUTO-UPDATE: a SILENT install is launched by the running Irori GUI (Go
    # or Rust: the same rules), which then quits. Two things must hold before
    # File writes irori.exe:
    #
    # 1. The GUI process is gone, so its single-instance lock is released and
    #    the relaunch at the end does not hand off to a dying instance. The GUI
    #    passes /WAITPID=<its pid> and we wait on that process handle (up to
    #    60 s). Without /WAITPID we fall back to waiting on the file lock.
    #
    # 2. irori.exe is writable. The GUI is NOT the only process running it:
    #    every agent session runs `irori mcp`, and `irori serve --headless` may
    #    be up, and each holds the exe image locked for as long as it runs. So
    #    after the GUI is gone we probe briefly and, if the exe is still locked,
    #    rename it aside to irori.exe.old (Windows allows renaming a running
    #    exe; the running processes keep their image) and write the new exe
    #    under the freed name. Old copies are deleted below on the next install
    #    and by the app at startup (irori_update::nsis::cleanup_old_exes) once
    #    nothing holds them.
    #
    # The PROBE opens the exe for writing (append, no truncation), which is what
    # File will do. A rename is no good as a probe: it succeeds while the old
    # process is still shutting down. Renaming as the SWAP is fine, because by
    # then we know the GUI has exited.
    #
    # If the exe can be neither written nor renamed, change nothing: relaunch
    # the old Irori unless /NORELAUNCH, exit with code 3, and the update is
    # offered again at the next start.
    ${If} ${Silent}
        ${GetParameters} $R0
        ClearErrors
        ${GetOptions} $R0 "/WAITPID=" $R2
        ${IfNot} ${Errors}
        ${AndIf} $R2 != ""
            # SYNCHRONIZE = 0x00100000. A failed open means the pid is gone.
            System::Call 'kernel32::OpenProcess(i 0x00100000, i 0, i $R2) p .R3'
            ${If} $R3 P<> 0
                System::Call 'kernel32::WaitForSingleObject(p R3, i ${IRORI_WAITPID_MS}) i .R4'
                System::Call 'kernel32::CloseHandle(p R3)'
                ${If} $R4 <> 0
                    # WAIT_TIMEOUT / WAIT_FAILED: the GUI never exited.
                    Goto irori_gave_up
                ${EndIf}
            ${EndIf}
            # The GUI has exited; give the loader a moment to drop its mapping.
            StrCpy $5 10
        ${Else}
            # No pid: wait on the file lock itself, as long as before.
            StrCpy $5 300
        ${EndIf}

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
            IntCmp $0 $5 irori_swap 0 irori_swap
            Sleep 200
            Goto irori_wait_unlock

        # Still locked by `irori mcp` / `irori serve`: rename it aside. Clear
        # out copies from earlier swaps first; one still running cannot be
        # deleted, so try .old, .old1 ... .old9 until a name is free.
        irori_swap:
            Delete "$INSTDIR\${PRODUCT_EXECUTABLE}.old*"
            StrCpy $6 0
        irori_swap_try:
            StrCpy $7 "$INSTDIR\${PRODUCT_EXECUTABLE}.old"
            ${If} $6 <> 0
                StrCpy $7 "$INSTDIR\${PRODUCT_EXECUTABLE}.old$6"
            ${EndIf}
            ClearErrors
            Rename "$INSTDIR\${PRODUCT_EXECUTABLE}" "$7"
            IfErrors 0 irori_unlocked
            IntOp $6 $6 + 1
            IntCmp $6 10 irori_gave_up irori_swap_try irori_gave_up

        irori_gave_up:
            Call IroriRelaunchUnlessNoRelaunch
            SetErrorLevel 3
            Abort "irori.exe is in use and could not be replaced; update skipped"
        irori_unlocked:
    ${EndIf}

    # Leftovers of an earlier swap whose holders have since exited.
    Delete "$INSTDIR\${PRODUCT_EXECUTABLE}.old*"

    SetOutPath $INSTDIR

    File "/oname=${PRODUCT_EXECUTABLE}" "${IRORI_BINARY}"

    !ifdef IRORI_SCRATCH
        # scripts/installer-check.ps1 reads what the shortcut step would see.
        FileOpen $0 "$INSTDIR\fresh-install.txt" w
        FileWrite $0 $FreshInstall
        FileClose $0
    !endif

    !ifndef IRORI_SCRATCH
        # The Start-menu shortcut is always (re)created; the Desktop one only
        # on a fresh install.
        CreateShortcut "$SMPROGRAMS\${INFO_PRODUCTNAME}.lnk" "$INSTDIR\${PRODUCT_EXECUTABLE}"
        ${If} $FreshInstall == "1"
            CreateShortCut "$DESKTOP\${INFO_PRODUCTNAME}.lnk" "$INSTDIR\${PRODUCT_EXECUTABLE}"
        ${EndIf}
    !endif

    WriteUninstaller "$INSTDIR\uninstall.exe"
    !ifndef IRORI_SCRATCH
        SetRegView 64
        WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "${INFO_COMPANYNAME}"
        WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${INFO_PRODUCTNAME}"
        WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${INFO_PRODUCTVERSION}"
        WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\${PRODUCT_EXECUTABLE}"
        WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" "$\"$INSTDIR\uninstall.exe$\""
        WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" "$\"$INSTDIR\uninstall.exe$\" /S"
        ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
        IntFmt $0 "0x%08X" $0
        WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" "$0"
    !endif

    # AUTO-UPDATE: after a silent update, relaunch the new Irori UNLESS the
    # updater passed /NORELAUNCH:
    #   "Restart now"           -> installer /S /WAITPID=<pid>              (relaunch)
    #   quit with update ready  -> installer /S /NORELAUNCH /WAITPID=<pid>  (stay closed)
    ${If} ${Silent}
        Call IroriRelaunchUnlessNoRelaunch
    ${EndIf}
SectionEnd

Section "uninstall"
    SetShellVarContext current

    !ifndef IRORI_SCRATCH
        # WebView2 data: Wails' (the Go build) and Tauri's (this build).
        RMDir /r "$AppData\${PRODUCT_EXECUTABLE}"
        RMDir /r "$LOCALAPPDATA\${TAURI_IDENTIFIER}"
    !endif

    RMDir /r $INSTDIR

    !ifndef IRORI_SCRATCH
        Delete "$SMPROGRAMS\${INFO_PRODUCTNAME}.lnk"
        Delete "$DESKTOP\${INFO_PRODUCTNAME}.lnk"

        # Launch at login (the Wails value both builds use) must not outlive
        # the app.
        DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Irori"

        SetRegView 64
        DeleteRegKey HKCU "${UNINST_KEY}"
    !endif
SectionEnd
