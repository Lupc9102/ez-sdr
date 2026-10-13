#ifndef AppVersion
  #error AppVersion is required
#endif
#ifndef SourceDir
  #error SourceDir is required
#endif
#ifndef OutputDir
  #error OutputDir is required
#endif

[Setup]
AppId={{A0C1F2D5-BCA8-4B98-96C4-8BD1F45F7D93}
AppName=EZ-SDR
AppVersion={#AppVersion}
AppPublisher=Lupc9102
DefaultDirName={autopf}\EZ-SDR
DefaultGroupName=EZ-SDR
UninstallDisplayIcon={app}\bin\ez-gui.exe
OutputDir={#OutputDir}
OutputBaseFilename=ez-sdr-v{#AppVersion}-windows-x86_64-setup
ArchitecturesInstallIn64BitMode=x64
ArchitecturesAllowed=x64
Compression=lzma2
SolidCompression=yes
PrivilegesRequired=admin
LicenseFile={#SourceDir}\LICENSES\GPL-3.0.txt
InfoBeforeFile={#SourceDir}\DISTRIBUTION.md

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: recursesubdirs ignoreversion

[Icons]
Name: "{group}\EZ-SDR"; Filename: "{app}\bin\ez-gui.exe"; WorkingDir: "{app}\bin"
Name: "{autodesktop}\EZ-SDR"; Filename: "{app}\bin\ez-gui.exe"; WorkingDir: "{app}\bin"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional icons:"; Flags: unchecked

[Run]
Filename: "{app}\bin\ez-gui.exe"; Description: "Launch EZ-SDR"; Flags: postinstall nowait skipifsilent
