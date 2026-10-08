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
; Démarrage avec Windows : raccourci du dossier Démarrage (même code que la case du
; menu de l'icône). Retiré automatiquement à la désinstallation.
Name: "{userstartup}\Clipper"; Filename: "{app}\clipper.exe"; Tasks: startup

[Registry]
; État « activé » dans Gestionnaire des tâches > Applications de démarrage.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\StartupFolder"; ValueType: binary; ValueName: "Clipper.lnk"; ValueData: "02 00 00 00 00 00 00 00 00 00 00 00"; Tasks: startup
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\StartupFolder"; ValueType: none; ValueName: "Clipper.lnk"; Flags: uninsdeletevalue
; Anciennes versions : démarrage par la clé Run, remplacée par le dossier Démarrage.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "Clipper"; Flags: deletevalue uninsdeletevalue
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"; ValueType: none; ValueName: "Clipper"; Flags: deletevalue uninsdeletevalue

[Run]
Filename: "{app}\clipper.exe"; Description: "Lancer Clipper"; Flags: nowait postinstall

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/im clipper.exe /f"; Flags: runhidden; RunOnceId: "StopClipper"

[UninstallDelete]
; La config est créée par Clipper au premier lancement ; les clips (Vidéos\Clipper) restent.
Type: files; Name: "{app}\clipper.toml"
