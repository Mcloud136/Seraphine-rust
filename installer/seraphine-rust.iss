#define AppVersion "2.0.2"
#define AppName "Seraphine Rust"
#define AppExe "seraphine-app.exe"
#define SourceExe "..\rust\app\src-tauri\target\release\seraphine-app.exe"

[Setup]
AppId={{8f3c2a1e-6b7d-4c5e-9a0f-2d1b3c4e5f67}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Mcloud
AppPublisherURL=https://gitee.com/wxbns/seraphine-rust
AppSupportURL=https://gitee.com/wxbns/seraphine-rust/issues
AppUpdatesURL=https://gitee.com/wxbns/seraphine-rust/releases
AppComments=英雄联盟战绩查询与辅助工具（Rust/Tauri 版）
AppContact=Mcloud
VersionInfoVersion={#AppVersion}.0
VersionInfoProductName={#AppName}
VersionInfoProductVersion={#AppVersion}
VersionInfoCopyright=Copyright © 2026 Mcloud
DefaultDirName={localappdata}\Programs\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
ShowLanguageDialog=no
LicenseFile=eula.zh-CN.txt
OutputDir=Output
OutputBaseFilename=SeraphineRust-{#AppVersion}-setup-zh
SetupIconFile=..\rust\app\src-tauri\icons\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
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
Name: "chinesesimplified"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: checkedonce

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "{#AppExe}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{group}\卸载 {#AppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Code]
function WebView2Installed: Boolean;
begin
  Result :=
    RegKeyExists(HKLM, 'SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}') or
    RegKeyExists(HKLM, 'SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}') or
    RegKeyExists(HKCU, 'Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}');
end;

function InitializeSetup: Boolean;
begin
  Result := True;
  if not WebView2Installed then
  begin
    MsgBox('未检测到 Microsoft Edge WebView2 运行时。' + #13#10 +
      'Seraphine Rust 依赖 WebView2 显示界面，请先从微软官网安装 ' +
      '“Microsoft Edge WebView2 Runtime” 后再运行安装程序。',
      mbError, MB_OK);
    Result := False;
  end;
end;