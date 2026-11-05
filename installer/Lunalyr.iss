#define ProductVersion "0.2.0"
#ifndef PublishDir
  #define PublishDir "..\artifacts\win-x64"
#endif
#ifndef OutputDir
  #define OutputDir "..\artifacts\installer"
#endif

[Setup]
AppId=WaterBotol.Lunalyr.Windows.2
AppName=Lunalyr for Windows
AppVersion={#ProductVersion}
DefaultDirName={localappdata}\Programs\LunalyrWindows2
DefaultGroupName=Lunalyr
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
MinVersion=10.0.19041
OutputDir={#OutputDir}
OutputBaseFilename=Lunalyr-Windows-x64-Setup
SetupIconFile=..\app\lunalyr.ico
UninstallDisplayIcon={app}\Lunalyr.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
LicenseFile=..\LICENSE

[Files]
Source: "{#PublishDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Lunalyr for Windows"; Filename: "{app}\Lunalyr.exe"

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "LunalyrWindows2"; Flags: dontcreatekey uninsdeletevalue

[Run]
Filename: "{app}\Lunalyr.exe"; Description: "Open Lunalyr"; Flags: nowait postinstall skipifsilent
