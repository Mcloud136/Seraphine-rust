#ifndef AppSourceDir
  #define AppSourceDir "..\dist\Seraphine"
#endif

#ifndef InstallerOutputDir
  #define InstallerOutputDir "..\dist\installer"
#endif

[Setup]
AppId={{15bc63d1-7fb5-5a22-88a9-cbb12cb8a348}
AppName=Seraphine
AppVersion=1.3.0
AppPublisher=Mcloud
AppPublisherURL=https://github.com/Mcloud136/Seraphine-rust
AppSupportURL=https://github.com/Mcloud136/Seraphine-rust/issues
AppUpdatesURL=https://github.com/Mcloud136/Seraphine-rust/releases
AppComments=League of Legends companion application
AppContact=Mcloud
VersionInfoVersion=1.3.0.0
VersionInfoProductName=Seraphine
VersionInfoCopyright=Copyright © 2026 Mcloud
DefaultDirName={localappdata}\Programs\Seraphine
DefaultGroupName=Seraphine
DisableProgramGroupPage=yes
LicenseFile=..\LICENSE
OutputDir={#InstallerOutputDir}
OutputBaseFilename=Seraphine-1.3.0-setup
SetupIconFile=..\app\resource\images\logo.ico
UninstallDisplayIcon={app}\Seraphine.exe
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
PrivilegesRequired=lowest
CloseApplications=yes
RestartApplications=no
SetupLogging=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: checkedonce

[Files]
Source: "{#AppSourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\Seraphine"; Filename: "{app}\Seraphine.exe"
Name: "{group}\Uninstall Seraphine"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Seraphine"; Filename: "{app}\Seraphine.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\Seraphine.exe"; Description: "{cm:LaunchProgram,Seraphine}"; Flags: nowait postinstall skipifsilent
