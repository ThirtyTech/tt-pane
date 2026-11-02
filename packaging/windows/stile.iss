; Compile through package.ps1. Keep AppId stable so upgrades replace sTile.
#ifndef AppId
  #define AppId "{{05CF136A-FC31-43EE-A704-44900F4B94F1}"
#endif
#ifndef AppVersion
  #error AppVersion is required
#endif
#ifndef ShortcutName
  #define ShortcutName "sTile"
#endif

[Setup]
AppId={#AppId}
AppName=sTile
AppVersion={#AppVersion}
AppVerName=sTile {#AppVersion}
AppPublisher=jsheely
AppPublisherURL=https://github.com/jsheely/sTile
AppSupportURL=https://github.com/jsheely/sTile/issues
AppUpdatesURL=https://github.com/jsheely/sTile/releases
VersionInfoVersion={#FileVersion}
DefaultDirName={localappdata}\Programs\sTile
DefaultGroupName={#ShortcutName}
DisableProgramGroupPage=yes
DisableWelcomePage=no
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputBaseFilename={#OutputBase}
SetupIconFile=..\..\assets\stile.ico
UninstallDisplayIcon={app}\stile.exe
UninstallDisplayName=sTile
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
WizardImageFile=artwork\sidebar.bmp,artwork\sidebar-2x.bmp
WizardSmallImageFile=artwork\small.bmp,artwork\small-2x.bmp
CloseApplications=yes
RestartApplications=no
SetupLogging=yes

[Messages]
WelcomeLabel1=Welcome to sTile
WelcomeLabel2=Put every window in its place.%n%nSetup will install sTile {#AppVersion} for your Windows account.%n%nUse keyboard shortcuts to arrange windows on any monitor. sTile runs quietly in your system tray; open Settings from its tray menu.%n%nQuit an existing sTile instance before updating.
FinishedLabel=Setup has installed sTile on your computer.%n%nFind sTile in your system tray. Open Settings from its tray menu to customize your shortcuts.%n%nStart with Ctrl+Alt+1 for the left half and Ctrl+Alt+2 for the right half.

[Tasks]
Name: desktopicon; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "Shortcuts:"; Flags: unchecked
Name: startup; Description: "Start sTile when I sign in to Windows"; GroupDescription: "Startup:"; Flags: unchecked

[Files]
Source: "{#StageDir}\stile.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#ShortcutName}"; Filename: "{app}\stile.exe"; Comment: "Keyboard-first window positioning"
Name: "{userdesktop}\{#ShortcutName}"; Filename: "{app}\stile.exe"; Tasks: desktopicon
Name: "{userstartup}\{#ShortcutName}"; Filename: "{app}\stile.exe"; Tasks: startup

; Remove shortcuts when their task is deselected during an upgrade.
[InstallDelete]
Type: files; Name: "{userdesktop}\{#ShortcutName}.lnk"; Tasks: not desktopicon
Type: files; Name: "{userstartup}\{#ShortcutName}.lnk"; Tasks: not startup

[Run]
Filename: "{app}\stile.exe"; Description: "Launch sTile in the system tray"; Flags: nowait postinstall skipifsilent unchecked

; Deliberately no UninstallDelete: configuration and cycle state belong to users.
