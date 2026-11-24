; Inno Setup 6/7 Installer Script for Better_Jarvis
; Produces zero-UAC, per-user 64-bit installer in %LOCALAPPDATA%\Programs\Better_Jarvis

#define MyAppName "Better_Jarvis"
#define MyAppVersion "1.0.0"
#define MyAppPublisher "Better_Jarvis Engineering"
#define MyAppURL "https://github.com/Youssef/Better_Jarvis"
#define MyAppExeName "jarvis-host.exe"

[Setup]
AppId={{C742D455-8AE5-4A9B-983F-311D7BB0A102}}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={localappdata}\Programs\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputBaseFilename=Better_Jarvis-v{#MyAppVersion}-Setup
OutputDir=..\dist
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no
UninstallDisplayIcon={app}\{#MyAppExeName}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "autostart"; Description: "Start Better_Jarvis automatically at user logon (Task Scheduler)"; GroupDescription: "Logon Options:"; Flags: checkedonce

[Files]
; Copy all staged standalone release files recursively
Source: "..\dist\Better_Jarvis-v1.0.0-win64\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{group}\Uninstall {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
; Optional logon task scheduler registration
Filename: "powershell.exe"; Parameters: "-ExecutionPolicy Bypass -NoProfile -File ""{app}\scripts\install-startup.ps1"" -InstallDir ""{app}"""; Tasks: autostart; Flags: runhidden
; Optional immediate application launch after setup
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Unregister scheduled task and stop running processes cleanly before files are removed
Filename: "powershell.exe"; Parameters: "-ExecutionPolicy Bypass -NoProfile -File ""{app}\scripts\uninstall-startup.ps1"""; Flags: runhidden
