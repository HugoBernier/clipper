; Installeur Clipper (Inno Setup 6). Prérequis : cargo build --release.
; Compiler : "%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe" installer\clipper.iss
; Sortie : target\installer\ClipperSetup-<version>.exe

; Version lue dans les informations de version de l'exe (elles-mêmes issues de Cargo.toml).
#define Exe "..\target\release\clipper.exe"
#define Version GetStringFileInfo(Exe, "ProductVersion")

[Setup]
; Identifiant stable : ne jamais le changer, il relie les mises à jour à l'installation.
AppId={{DD577186-5721-4E2C-9A75-FB10E9074CE5}
AppName=Clipper
AppVersion={#Version}
AppVerName=Clipper {#Version}
; Par utilisateur, sans droits admin : {autopf} = %LOCALAPPDATA%\Programs.
PrivilegesRequired=lowest
DefaultDirName={autopf}\Clipper
DisableProgramGroupPage=yes
DisableDirPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\target\installer
OutputBaseFilename=ClipperSetup-{#Version}
SetupIconFile=..\assets\clipper.ico
UninstallDisplayIcon={app}\clipper.exe
WizardStyle=modern
Compression=lzma2
SolidCompression=yes
; Clipper n'a pas de fenêtre : on force sa fermeture pour remplacer l'exe.
CloseApplications=force
RestartApplications=no

[Languages]
Name: "french"; MessagesFile: "compiler:Languages\French.isl"

[Tasks]
Name: "startup"; Description: "Démarrer Clipper avec Windows"

[Files]
Source: "{#Exe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Clipper"; Filename: "{app}\clipper.exe"

[Registry]
; Même valeur que la case « Démarrer avec Windows » du menu de l'icône.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Clipper"; ValueData: """{app}\clipper.exe"""; Tasks: startup
; Retirée à la désinstallation même si elle a été activée depuis le menu de l'icône.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "Clipper"; Flags: uninsdeletevalue

[Run]
Filename: "{app}\clipper.exe"; Description: "Lancer Clipper"; Flags: nowait postinstall

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/im clipper.exe /f"; Flags: runhidden; RunOnceId: "StopClipper"

[UninstallDelete]
; La config est créée par Clipper au premier lancement ; les clips (Vidéos\Clipper) restent.
Type: files; Name: "{app}\clipper.toml"
