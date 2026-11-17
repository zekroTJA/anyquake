; Inno Setup script for the AnyQuake installer, built from an already built
; binary:
;
;   iscc /DAppVersion=<version> packaging\windows\anyquake.iss
;
; Writes dist\AnyQuake-<version>-<arch>-setup.exe. /DArch=arm64 builds the
; installer for an ARM64 binary (default: x64). /DBinary=<path> uses another
; binary than target\release\anyquake.exe, /O<dir> another output directory.
; Relative paths are relative to this file.

#ifndef AppVersion
  #error Pass the version: iscc /DAppVersion=<version>
#endif
#ifndef Arch
  #define Arch "x64"
#endif
#if Arch == "x64"
  ; Also installs on Windows on ARM, where the x64 binary runs emulated.
  #define ArchAllowed "x64compatible"
#elif Arch == "arm64"
  #define ArchAllowed "arm64"
#else
  #error Arch must be x64 or arm64
#endif
#ifndef Binary
  #define Binary "..\..\target\release\anyquake.exe"
#endif

[Setup]
; Identifies the installation for upgrades and uninstall. Never change it.
AppId={{74E458BE-0398-4DB1-BEF7-D20D32C872C4}
AppName=AnyQuake
AppVersion={#AppVersion}
AppPublisher=Ringo Hoffmann
AppComments=Toggle any application window on the current screen with a global hotkey
DefaultDirName={autopf}\AnyQuake
DefaultGroupName=AnyQuake
DisableProgramGroupPage=yes
; Installs for the current user without administrator rights by default
; (%LOCALAPPDATA%\Programs\AnyQuake), like the autostart entry, which is
; per user too. The dialog offers installing for all users instead.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
UsedUserAreasWarning=no
ArchitecturesAllowed={#ArchAllowed}
ArchitecturesInstallIn64BitMode={#ArchAllowed}
MinVersion=10.0
SetupIconFile=anyquake.ico
UninstallDisplayIcon={app}\anyquake.ico
UninstallDisplayName=AnyQuake
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
OutputDir=..\..\dist
OutputBaseFilename=AnyQuake-{#AppVersion}-{#Arch}-setup

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Binary}"; DestDir: "{app}"; DestName: "anyquake.exe"; Flags: ignoreversion
Source: "anyquake.ico"; DestDir: "{app}"
Source: "..\..\README.md"; DestDir: "{app}"
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"

[Icons]
Name: "{autoprograms}\AnyQuake"; Filename: "{app}\anyquake.exe"; IconFilename: "{app}\anyquake.ico"; Comment: "AnyQuake settings"
Name: "{autodesktop}\AnyQuake"; Filename: "{app}\anyquake.exe"; IconFilename: "{app}\anyquake.ico"; Comment: "AnyQuake settings"; Tasks: desktopicon

[Registry]
; The autostart entry AnyQuake writes when it is enabled in the settings
; (src/autostart.rs, auto-launch crate). Only removed on uninstall.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueName: "anyquake"; ValueType: none; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"; ValueName: "anyquake"; ValueType: none; Flags: uninsdeletevalue

[Run]
Filename: "{app}\anyquake.exe"; Description: "{cm:LaunchProgram,AnyQuake}"; Flags: nowait postinstall skipifsilent runasoriginaluser

[Code]
// Stops a running AnyQuake so its executable can be replaced or removed.
// Quitting also gives the managed window back. `anyquake quit` fails once
// no instance answers anymore.
procedure StopAnyQuake();
var
  Exe: String;
  ResultCode, I: Integer;
  WasRunning: Boolean;
begin
  Exe := ExpandConstant('{app}\anyquake.exe');
  if not FileExists(Exe) then
    Exit;
  WasRunning := False;
  for I := 1 to 50 do
  begin
    if not Exec(Exe, 'quit', '', SW_HIDE, ewWaitUntilTerminated, ResultCode) or (ResultCode <> 0) then
      Break;
    WasRunning := True;
    Sleep(100);
  end;
  // The process may still be exiting after its control socket is gone.
  if WasRunning then
    Sleep(500);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopAnyQuake();
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    StopAnyQuake();
end;
