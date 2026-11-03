; Compile through package.ps1. Keep AppId stable so upgrades replace tt-pane.
#ifndef AppId
  #define AppId "{{05CF136A-FC31-43EE-A704-44900F4B94F1}"
  #define LegacyUpgrade
#endif
#ifndef AppVersion
  #error AppVersion is required
#endif
#ifndef ShortcutName
  #define ShortcutName "tt-pane"
#endif

[Setup]
AppId={#AppId}
AppName=tt-pane
AppVersion={#AppVersion}
AppVerName=tt-pane {#AppVersion}
AppPublisher=ThirtyTech
AppPublisherURL=https://github.com/thirtytech/tt-pane
AppSupportURL=https://github.com/thirtytech/tt-pane/issues
AppUpdatesURL=https://github.com/thirtytech/tt-pane/releases
VersionInfoVersion={#FileVersion}
DefaultDirName={localappdata}\Programs\tt-pane
DefaultGroupName={#ShortcutName}
DisableProgramGroupPage=yes
DisableWelcomePage=no
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputBaseFilename={#OutputBase}
SetupIconFile=..\..\assets\tt-pane.ico
UninstallDisplayIcon={app}\tt-pane.exe
UninstallDisplayName=tt-pane
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
WizardImageFile=artwork\sidebar.bmp,artwork\sidebar-2x.bmp
WizardSmallImageFile=artwork\small.bmp,artwork\small-2x.bmp
CloseApplications=yes
RestartApplications=no
SetupLogging=yes

[Messages]
WelcomeLabel1=Welcome to tt-pane
WelcomeLabel2=Put every window in its place.%n%nSetup will install tt-pane {#AppVersion} for your Windows account.%n%nUse keyboard shortcuts to arrange windows on any monitor. tt-pane runs quietly in your system tray; open Settings from its tray menu.%n%nQuit an existing tt-pane instance before updating.
FinishedLabel=Setup has installed tt-pane on your computer.%n%nFind tt-pane in your system tray. Open Settings from its tray menu to customize your shortcuts.%n%nStart with Ctrl+Alt+1 for the left half and Ctrl+Alt+2 for the right half.

[Tasks]
Name: desktopicon; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "Shortcuts:"; Flags: unchecked
Name: startup; Description: "Start tt-pane when I sign in to Windows"; GroupDescription: "Startup:"; Flags: unchecked

[Files]
Source: "{#StageDir}\tt-pane.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#ShortcutName}"; Filename: "{app}\tt-pane.exe"; Comment: "Keyboard-first window positioning"
Name: "{userdesktop}\{#ShortcutName}"; Filename: "{app}\tt-pane.exe"; Tasks: desktopicon
Name: "{userstartup}\{#ShortcutName}"; Filename: "{app}\tt-pane.exe"; Tasks: startup

; Remove shortcuts when their task is deselected during an upgrade.
[InstallDelete]
#ifdef LegacyUpgrade
; Remove only the former application's executable and shortcuts on upgrade.
Type: files; Name: "{app}\stile.exe"
Type: files; Name: "{userprograms}\sTile\sTile.lnk"
Type: dirifempty; Name: "{userprograms}\sTile"
Type: files; Name: "{userdesktop}\sTile.lnk"
Type: files; Name: "{userstartup}\sTile.lnk"
#endif
Type: files; Name: "{userdesktop}\{#ShortcutName}.lnk"; Tasks: not desktopicon
Type: files; Name: "{userstartup}\{#ShortcutName}.lnk"; Tasks: not startup

[Run]
Filename: "{app}\tt-pane.exe"; Description: "Launch tt-pane in the system tray"; Flags: nowait postinstall skipifsilent unchecked

; Deliberately no UninstallDelete: configuration and cycle state belong to users.
