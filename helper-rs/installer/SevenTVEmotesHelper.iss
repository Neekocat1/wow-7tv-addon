; Inno Setup 6 script for the 7TV Emotes Helper (Rust).
;
; Replaces the Python helper's "copy myself to %LOCALAPPDATA% and add a Run key" first-run step,
; a pattern antivirus heuristics associate with malware. A regular per-user installer:
;   - puts the exe in %LOCALAPPDATA%\Programs\SevenTVEmotesHelper (no admin rights needed)
;   - adds the "start with Windows" entry (HKCU Run, removed again on uninstall)
;   - asks a running helper (Rust or Python) to quit first, removes the old Python copy
;   - starts the helper at the end, which installs the addons into WoW on its first run
;
; Build: build.bat (or: ISCC installer\SevenTVEmotesHelper.iss after cargo build --release)
; Output: dist\SevenTVEmotesHelper-Setup-<version>.exe

#define AppName "7TV Emotes Helper"
#define AppExe "SevenTVEmotesHelper.exe"
#define AppExeSource "..\target\release\SevenTVEmotesHelper.exe"
; The version comes from the exe (Cargo.toml -> version resource), e.g. "0.6.0.0".
#define AppVersion GetVersionNumbersString(AppExeSource)

[Setup]
AppId={{6F1D7A52-3C1B-4E57-9B3A-7E0C2D5B9A41}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Connor
VersionInfoVersion={#AppVersion}
VersionInfoDescription={#AppName} Setup
DefaultDirName={localappdata}\Programs\SevenTVEmotesHelper
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=SevenTVEmotesHelper-Setup-{#AppVersion}
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; We stop the helper ourselves (see [Code]); the Restart Manager prompt would only confuse.
CloseApplications=no

[Files]
Source: "{#AppExeSource}"; DestDir: "{app}"; Flags: ignoreversion

[Registry]
; Same value the helper's tray toggle ("Start with Windows") and --autostart switch.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "SevenTVEmotesHelper"; ValueData: """{app}\{#AppExe}"" --background"; Flags: uninsdeletevalue

[InstallDelete]
; The Python helper (0.5 and earlier) copied itself here; the Run key above replaces its entry.
Type: files; Name: "{localappdata}\SevenTVEmotesHelper\SevenTVEmotesHelper.exe"
Type: files; Name: "{localappdata}\SevenTVEmotesHelper\installed_version"

[UninstallDelete]
; The helper's own folder: config, log, download cache. Emotes inside WoW are left alone.
Type: filesandordirs; Name: "{localappdata}\SevenTVEmotesHelper"

[Run]
Filename: "{app}\{#AppExe}"; Description: "Start the {#AppName} now"; Flags: nowait postinstall

[Code]
// Ask a running helper to quit, the same way a newer helper does: create quit.request in the
// helper's app folder. Rust helpers hold the named mutex below while running; Python helpers
// (0.5.x) have no mutex but check for quit.request every 2 seconds.
procedure AskHelperToQuit();
var
  Dir: String;
  I: Integer;
begin
  Dir := ExpandConstant('{localappdata}\SevenTVEmotesHelper');
  ForceDirectories(Dir);
  SaveStringToFile(Dir + '\quit.request', '{#AppVersion}', False);
  I := 0;
  while CheckForMutexes('SevenTVEmotesHelper_running') and (I < 60) do
  begin
    Sleep(250);
    I := I + 1;
  end;
  Sleep(2500);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  AskHelperToQuit();
  // The helper started at the end of setup clears quit.request when it takes over.
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    AskHelperToQuit();
end;
