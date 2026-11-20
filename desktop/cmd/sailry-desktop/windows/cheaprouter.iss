; CheapRouter (Go/MyGo) Windows installer.
;
; Reuses main's Inno Setup AppId and per-user install contract so first-hop
; silent updates from Rust/GPUI v0.3.x (/SILENT /NORESTART /SP- /DIR=...)
; replace the existing installation in place without UAC prompts or duplicate
; Add/Remove Programs entries, clean legacy Rust executables, and launch
; CheapRouter.exe.
;
; Built by desktop/cmd/sailry-desktop/windows/package.sh with:
;   /DAppVersion=<version>  /DArch=<x86_64|aarch64>
;   /DStageDir=<dir with CheapRouter.exe and shipped runtime>
;   /DOutputDir=<dir>  /DIconFile=<path to .ico>  /DLicenseFile=<path to LICENSE>
;   /DAppUserModelID=<AppUserModelID matching SetCurrentProcessExplicitAppUserModelID>

#ifndef AppVersion
  #error AppVersion must be defined (ISCC /DAppVersion=...)
#endif
#ifndef Arch
  #error Arch must be defined (ISCC /DArch=...)
#endif
#ifndef StageDir
  #error StageDir must be defined (ISCC /DStageDir=...)
#endif
#ifndef OutputDir
  #define OutputDir "."
#endif
#ifndef IconFile
  #define IconFile "..\..\..\assets\branding\cheaprouter.ico"
#endif
#ifndef LicenseFile
  #define LicenseFile "..\..\..\..\LICENSE"
#endif
#ifndef AppUserModelID
  #define AppUserModelID "sh.waku"
#endif

#if Arch == "aarch64"
  #define Architectures "arm64"
#else
  #define Architectures "x64compatible"
#endif

[Setup]
; Must stay identical to resources/windows/waku.iss so first-hop upgrades
; replace the existing installation in place instead of creating a second copy.
AppId={{7DC6C35B-FA40-4A95-B37A-626BF64556C5}
AppName=CheapRouter
AppVersion={#AppVersion}
VersionInfoVersion={#AppVersion}
AppPublisher=CheapRouter
AppPublisherURL=https://cheaprouter.cc
AppSupportURL=https://github.com/ai-poet/agent-client/issues
AppUpdatesURL=https://github.com/ai-poet/agent-client/releases
DefaultDirName={autopf}\CheapRouter
DefaultGroupName=CheapRouter
UninstallDisplayName=CheapRouter
UninstallDisplayIcon={app}\CheapRouter.exe,0
LicenseFile={#LicenseFile}
OutputDir={#OutputDir}
OutputBaseFilename=CheapRouter-{#AppVersion}-{#Arch}-Setup
SetupIconFile={#IconFile}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed={#Architectures}
ArchitecturesInstallIn64BitMode={#Architectures}
MinVersion=10.0.17763
SetupMutex=CheapRouterSetup
PrivilegesRequired=lowest
DisableProgramGroupPage=yes
DisableReadyPage=yes
UsePreviousAppDir=yes
CloseApplications=force
CloseApplicationsFilter=*.exe,*.dll
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[InstallDelete]
; Clean legacy Rust/GPUI main binaries and helpers when upgrading in place.
Type: files; Name: "{app}\waku.exe"
Type: files; Name: "{app}\client-daemon.exe"
Type: files; Name: "{app}\waku_js_repl.exe"
Type: filesandordirs; Name: "{app}\computer-use"
Type: filesandordirs; Name: "{app}\skills\waku-computer-use"

[Files]
Source: "{#StageDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\CheapRouter"; Filename: "{app}\CheapRouter.exe"; IconFilename: "{app}\CheapRouter.exe"; IconIndex: 0; AppUserModelID: "{#AppUserModelID}"
Name: "{userdesktop}\CheapRouter"; Filename: "{app}\CheapRouter.exe"; IconFilename: "{app}\CheapRouter.exe"; IconIndex: 0; AppUserModelID: "{#AppUserModelID}"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked

[Run]
; No skipifsilent: main's silent updater relies on this to launch the new app.
Filename: "{app}\CheapRouter.exe"; Description: "{cm:LaunchProgram,CheapRouter}"; Flags: nowait postinstall

[Code]
procedure StopRunningProcess(const ImageName: String);
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /T /IM ' + ImageName, '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  // Terminate background Rust daemon and app binaries before file replacement
  // so SQLite WAL handles are released cleanly and executables are unlocked.
  StopRunningProcess('client-daemon.exe');
  StopRunningProcess('waku_js_repl.exe');
  StopRunningProcess('waku.exe');
  StopRunningProcess('CheapRouter.exe');
  Result := '';
end;
