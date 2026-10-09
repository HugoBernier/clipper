//! Icône dans la zone de notification : info-bulle, clic gauche pour ouvrir la fenêtre,
//! menu (fenêtre, dossier des clips, micro, démarrage avec Windows, quitter).
//!
//! Une fenêtre cachée reçoit les messages de l'icône ; elle n'a pas de style
//! visible, donc ni fenêtre à l'écran ni bouton dans la barre des tâches.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use anyhow::{Context, Result, bail};
use log::{error, info, warn};
use windows::Win32::Foundation::{ERROR_SUCCESS, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, IPersistFile,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_BINARY, RRF_RT_REG_BINARY, RegDeleteKeyValueW, RegGetValueW,
    RegSetKeyValueW,
};
use windows::Win32::UI::Shell::{
    FOLDERID_Startup, IShellLinkW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFY_ICON_MESSAGE, NOTIFYICONDATAW, Shell_NotifyIconW, ShellLink,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, Interface, PCWSTR, w};

use crate::{save, ui};

/// Message envoyé par l'icône à la fenêtre cachée.
const WM_TRAY: u32 = WM_APP + 1;
/// Envoyé à la fenêtre principale pour couper ou réactiver le micro.
pub const WM_MICROPHONE: u32 = WM_APP + 2;
const ID_OPEN: usize = 1;
const ID_STARTUP: usize = 2;
const ID_QUIT: usize = 3;
const ID_MICROPHONE: usize = 4;
const ID_WINDOW: usize = 5;
/// Démarrage avec Windows : raccourci dans le dossier Démarrage (`shell:startup`), la
/// méthode documentée par Microsoft pour les applications de bureau (Telegram, Ollama
/// font de même).
const SHORTCUT: &str = "Clipper.lnk";
/// État affiché dans Gestionnaire des tâches > Applications de démarrage.
const APPROVED_KEY: PCWSTR =
    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\StartupFolder");
const APPROVED_VALUE: PCWSTR = w!("Clipper.lnk");
/// 1er octet pair = activé (02), impair = désactivé (03) ; le reste est un horodatage.
const APPROVED_ENABLED: [u8; 12] = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
/// Icône de l'application (assets/clipper.rc), aussi utilisée pour la notification et
/// la fenêtre.
pub const ICON_RESOURCE: u16 = 1;

/// État lu par la procédure de fenêtre (fonction `extern "system"` sans contexte).
static CLIPS_DIR: Mutex<PathBuf> = Mutex::new(PathBuf::new());
static TOOLTIP: Mutex<String> = Mutex::new(String::new());
static ICON: AtomicUsize = AtomicUsize::new(0);
/// Case « Micro » du menu.
static MICROPHONE: AtomicBool = AtomicBool::new(false);
/// Diffusé par l'Explorateur quand il redémarre : il faut remettre l'icône.
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

/// L'icône vit tant que cette valeur existe.
pub struct Tray {
    hwnd: HWND,
}

impl Tray {
    /// À créer sur le thread qui fait tourner la boucle de messages.
    pub fn new(clips_dir: PathBuf, tooltip: String) -> Result<Self> {
        set_clips_dir(clips_dir);
        *TOOLTIP.lock().unwrap_or_else(|e| e.into_inner()) = tooltip;
        // SAFETY: création classique d'une classe et d'une fenêtre Win32 cachée.
        unsafe {
            let instance = GetModuleHandleW(None).context("GetModuleHandleW")?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance.into(),
                lpszClassName: w!("ClipperTray"),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                bail!("RegisterClassW");
            }
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("ClipperTray"),
                w!("Clipper"),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .context("CreateWindowExW")?;
            TASKBAR_CREATED.store(
                RegisterWindowMessageW(w!("TaskbarCreated")),
                Ordering::Relaxed,
            );
            ICON.store(load_icon(instance.into())?.0 as usize, Ordering::Relaxed);
            // Zone de notification pas encore prête : la fenêtre reste, et l'icône sera
            // ajoutée au message « TaskbarCreated ».
            if let Err(e) = notify(hwnd, NIM_ADD) {
                warn!(
                    "icône pas encore ajoutée ({e:#}) ; nouvel essai quand l'Explorateur sera prêt"
                );
            }
            Ok(Self { hwnd })
        }
    }
}

impl Tray {
    pub fn set_tooltip(&self, tooltip: &str) {
        *TOOLTIP.lock().unwrap_or_else(|e| e.into_inner()) = tooltip.into();
        if let Err(e) = notify(self.hwnd, NIM_MODIFY) {
            warn!("info-bulle non mise à jour : {e:#}");
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        // Sans ça, l'icône reste affichée jusqu'au survol de la souris.
        let data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            ..Default::default()
        };
        // SAFETY: suppression de l'icône ajoutée par `add_icon`.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

pub fn set_microphone(enabled: bool) {
    MICROPHONE.store(enabled, Ordering::Relaxed);
}

pub fn set_clips_dir(dir: PathBuf) {
    *CLIPS_DIR.lock().unwrap_or_else(|e| e.into_inner()) = dir;
}

/// Ajoute l'icône (`NIM_ADD`) ou met à jour son info-bulle (`NIM_MODIFY`).
fn notify(hwnd: HWND, action: NOTIFY_ICON_MESSAGE) -> Result<()> {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: HICON(ICON.load(Ordering::Relaxed) as _),
        ..Default::default()
    };
    let tip: Vec<u16> = TOOLTIP
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .encode_utf16()
        .collect();
    let n = tip.len().min(data.szTip.len() - 1);
    data.szTip[..n].copy_from_slice(&tip[..n]);
    // SAFETY: structure complète, fenêtre valide.
    if !unsafe { Shell_NotifyIconW(action, &data) }.as_bool() {
        bail!("Shell_NotifyIconW");
    }
    Ok(())
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_TRAY {
        // Sans NIM_SETVERSION, lParam porte directement le message souris.
        let result = match lparam.0 as u32 {
            WM_LBUTTONUP => ui::open(),
            WM_RBUTTONUP => show_menu(hwnd),
            _ => Ok(()),
        };
        if let Err(e) = result {
            error!("icône : {e:#}");
        }
        return LRESULT(0);
    }
    if msg == TASKBAR_CREATED.load(Ordering::Relaxed) {
        if let Err(e) = notify(hwnd, NIM_ADD) {
            error!("icône non restaurée après redémarrage de l'Explorateur : {e:#}");
        }
        return LRESULT(0);
    }
    // SAFETY: traitement par défaut des autres messages.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn show_menu(hwnd: HWND) -> Result<()> {
    let startup = startup_enabled();
    // SAFETY: menu contextuel classique ; SetForegroundWindow avant TrackPopupMenu et
    // WM_NULL après sont requis pour que le menu se ferme en cliquant ailleurs.
    let choice = unsafe {
        let menu = CreatePopupMenu()?;
        AppendMenuW(menu, MF_STRING, ID_WINDOW, w!("Ouvrir Clipper")).context("AppendMenuW")?;
        AppendMenuW(menu, MF_STRING, ID_OPEN, w!("Ouvrir le dossier des clips"))?;
        AppendMenuW(
            menu,
            MF_STRING | checked(MICROPHONE.load(Ordering::Relaxed)),
            ID_MICROPHONE,
            w!("Micro"),
        )?;
        AppendMenuW(
            menu,
            MF_STRING | checked(startup),
            ID_STARTUP,
            w!("Démarrer avec Windows"),
        )?;
        AppendMenuW(menu, MF_SEPARATOR, 0, None)?;
        AppendMenuW(menu, MF_STRING, ID_QUIT, w!("Quitter"))?;
        let mut cursor = POINT::default();
        GetCursorPos(&mut cursor)?;
        let _ = SetForegroundWindow(hwnd);
        let choice = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            cursor.x,
            cursor.y,
            None,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        DestroyMenu(menu)?;
        choice.0 as usize
    };
    match choice {
        // La config appartient à l'état principal : on lui délègue.
        ID_MICROPHONE => crate::post_to_main(WM_MICROPHONE),
        ID_WINDOW => ui::open()?,
        ID_OPEN => {
            let dir = CLIPS_DIR.lock().unwrap_or_else(|e| e.into_inner()).clone();
            std::fs::create_dir_all(&dir)?;
            std::process::Command::new("explorer").arg(&dir).spawn()?;
        }
        ID_STARTUP => {
            set_startup(!startup)?;
            ui::refresh();
        }
        // SAFETY: termine la boucle de messages du thread principal.
        ID_QUIT => unsafe { PostQuitMessage(0) },
        _ => {}
    }
    Ok(())
}

fn checked(on: bool) -> MENU_ITEM_FLAGS {
    if on { MF_CHECKED } else { MF_UNCHECKED }
}

fn shortcut_path() -> Result<PathBuf> {
    Ok(save::known_folder(&FOLDERID_Startup)?.join(SHORTCUT))
}

/// Coché si le raccourci existe et n'est pas désactivé dans le Gestionnaire des tâches.
pub fn startup_enabled() -> bool {
    let mut state = [0u8; 12];
    let mut len = state.len() as u32;
    // SAFETY: lecture d'une valeur binaire HKCU dans un tampon de la taille annoncée.
    let found = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            APPROVED_KEY,
            APPROVED_VALUE,
            RRF_RT_REG_BINARY,
            None,
            Some(state.as_mut_ptr().cast()),
            Some(&mut len),
        )
    } == ERROR_SUCCESS;
    shortcut_path().is_ok_and(|p| p.exists()) && !is_disabled(found.then(|| &state[..len as usize]))
}

/// Désactivé seulement si le Gestionnaire des tâches l'a marqué (1er octet impair).
fn is_disabled(state: Option<&[u8]>) -> bool {
    state.and_then(<[u8]>::first).is_some_and(|b| b & 1 == 1)
}

pub fn set_startup(enabled: bool) -> Result<()> {
    let link = shortcut_path()?;
    if enabled {
        create_shortcut(&link, &std::env::current_exe()?)?;
        // SAFETY: écriture d'une valeur binaire HKCU, taille passée en octets.
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                APPROVED_KEY,
                APPROVED_VALUE,
                REG_BINARY.0,
                Some(APPROVED_ENABLED.as_ptr().cast()),
                APPROVED_ENABLED.len() as u32,
            )
        };
        if status != ERROR_SUCCESS {
            bail!("registre (StartupApproved) : erreur {}", status.0);
        }
    } else {
        if link.exists() {
            std::fs::remove_file(&link)?;
        }
        // SAFETY: suppression d'une valeur HKCU ; son absence n'est pas une erreur.
        let _ = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, APPROVED_KEY, APPROVED_VALUE) };
    }
    info!(
        "démarrage avec Windows : {}",
        if enabled { "activé" } else { "désactivé" }
    );
    Ok(())
}

/// Raccourci `.lnk` via l'API Shell (IShellLinkW + IPersistFile).
fn create_shortcut(link: &Path, target: &Path) -> Result<()> {
    // SAFETY: objets COM standard du Shell, créés et utilisés sur ce thread.
    unsafe {
        // Déjà initialisé sur ce thread : sans effet.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let shortcut: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).context("ShellLink")?;
        shortcut.SetPath(&HSTRING::from(target.as_os_str()))?;
        if let Some(dir) = target.parent() {
            shortcut.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
        }
        shortcut
            .cast::<IPersistFile>()?
            .Save(&HSTRING::from(link.as_os_str()), true)
            .context("enregistrement du raccourci")?;
    }
    Ok(())
}

/// Icône de l'exe, à la taille des petites icônes (suit le DPI).
fn load_icon(instance: windows::Win32::Foundation::HINSTANCE) -> Result<HICON> {
    // SAFETY: ressource embarquée par build.rs ; MAKEINTRESOURCE = id casté en pointeur.
    unsafe {
        let handle = LoadImageW(
            Some(instance),
            PCWSTR(ICON_RESOURCE as usize as *const u16),
            IMAGE_ICON,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            LR_DEFAULTCOLOR,
        )
        .context("LoadImageW (icône)")?;
        Ok(HICON(handle.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_is_disabled_only_when_task_manager_says_so() {
        assert!(!is_disabled(Some(&APPROVED_ENABLED)));
        assert!(!is_disabled(Some(&[6, 0, 0, 0])));
        assert!(is_disabled(Some(&[3, 0, 0, 0])));
        // Absente (raccourci posé à la main) : Windows le lance.
        assert!(!is_disabled(None));
        assert!(!is_disabled(Some(&[])));
    }
}
