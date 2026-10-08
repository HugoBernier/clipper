//! Icône dans la zone de notification : info-bulle et menu (dossier des clips,
//! démarrage avec Windows, quitter).
//!
//! Une fenêtre cachée reçoit les messages de l'icône ; elle n'a pas de style
//! visible, donc ni fenêtre à l'écran ni bouton dans la barre des tâches.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use anyhow::{Context, Result, bail};
use log::{error, info};
use windows::Win32::Foundation::{ERROR_SUCCESS, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_BINARY, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_SZ, RegDeleteKeyValueW,
    RegGetValueW, RegSetKeyValueW,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

/// Message envoyé par l'icône à la fenêtre cachée.
const WM_TRAY: u32 = WM_APP + 1;
const ID_OPEN: usize = 1;
const ID_STARTUP: usize = 2;
const ID_QUIT: usize = 3;
const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("Clipper");
/// État de l'entrée dans Gestionnaire des tâches > Applications de démarrage. Windows ne
/// lance pas une entrée `Run` absente de cette clé (constaté au reboot du 2026-10-08).
const APPROVED_KEY: PCWSTR =
    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run");
/// 1er octet pair = activé (02), impair = désactivé (03) ; le reste est un horodatage.
const APPROVED_ENABLED: [u8; 12] = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
/// Icône de l'application (assets/clipper.rc), aussi utilisée pour la notification.
const ICON_RESOURCE: u16 = 1;

/// État lu par la procédure de fenêtre (fonction `extern "system"` sans contexte).
static CLIPS_DIR: OnceLock<PathBuf> = OnceLock::new();
static TOOLTIP: OnceLock<String> = OnceLock::new();
static ICON: AtomicUsize = AtomicUsize::new(0);
/// Diffusé par l'Explorateur quand il redémarre : il faut remettre l'icône.
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

/// L'icône vit tant que cette valeur existe.
pub struct Tray {
    hwnd: HWND,
}

impl Tray {
    /// À créer sur le thread qui fait tourner la boucle de messages.
    pub fn new(clips_dir: PathBuf, tooltip: String) -> Result<Self> {
        let _ = CLIPS_DIR.set(clips_dir);
        let _ = TOOLTIP.set(tooltip);
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
            add_icon(hwnd)?;
            Ok(Self { hwnd })
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

fn add_icon(hwnd: HWND) -> Result<()> {
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
        .get()
        .map_or("Clipper", String::as_str)
        .encode_utf16()
        .collect();
    let n = tip.len().min(data.szTip.len() - 1);
    data.szTip[..n].copy_from_slice(&tip[..n]);
    // SAFETY: structure complète, fenêtre valide.
    if !unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool() {
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
        let mouse = lparam.0 as u32;
        if (mouse == WM_RBUTTONUP || mouse == WM_LBUTTONUP)
            && let Err(e) = show_menu(hwnd)
        {
            error!("menu de l'icône : {e:#}");
        }
        return LRESULT(0);
    }
    if msg == TASKBAR_CREATED.load(Ordering::Relaxed) {
        if let Err(e) = add_icon(hwnd) {
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
        AppendMenuW(menu, MF_STRING, ID_OPEN, w!("Ouvrir le dossier des clips"))?;
        let check = if startup { MF_CHECKED } else { MF_UNCHECKED };
        AppendMenuW(
            menu,
            MF_STRING | check,
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
        ID_OPEN => {
            let dir = CLIPS_DIR.get().context("dossier des clips inconnu")?;
            std::fs::create_dir_all(dir)?;
            std::process::Command::new("explorer").arg(dir).spawn()?;
        }
        ID_STARTUP => {
            set_startup(!startup)?;
            info!(
                "démarrage avec Windows : {}",
                if startup { "désactivé" } else { "activé" }
            );
        }
        // SAFETY: termine la boucle de messages du thread principal.
        ID_QUIT => unsafe { PostQuitMessage(0) },
        _ => {}
    }
    Ok(())
}

/// Coché seulement si Windows lancera vraiment Clipper : valeur `Run` présente et
/// non désactivée dans les Applications de démarrage.
fn startup_enabled() -> bool {
    let mut state = [0u8; 12];
    let mut len = state.len() as u32;
    // SAFETY: lectures de valeurs HKCU ; le tampon binaire a la taille annoncée.
    let (run, approved) = unsafe {
        (
            RegGetValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                RUN_VALUE,
                RRF_RT_REG_SZ,
                None,
                None,
                None,
            ),
            RegGetValueW(
                HKEY_CURRENT_USER,
                APPROVED_KEY,
                RUN_VALUE,
                RRF_RT_REG_BINARY,
                None,
                Some(state.as_mut_ptr().cast()),
                Some(&mut len),
            ),
        )
    };
    run == ERROR_SUCCESS && is_approved((approved == ERROR_SUCCESS).then(|| &state[..len as usize]))
}

fn is_approved(state: Option<&[u8]>) -> bool {
    state.and_then(<[u8]>::first).is_some_and(|b| b & 1 == 0)
}

fn set_startup(enabled: bool) -> Result<()> {
    // SAFETY: écriture ou suppression de valeurs HKCU ; chaque tampon est passé avec
    // sa taille en octets (chaîne UTF-16 terminée par un zéro, ou binaire).
    unsafe {
        if enabled {
            let exe = std::env::current_exe()?;
            let value: Vec<u16> = run_command(&exe.to_string_lossy())
                .encode_utf16()
                .chain([0])
                .collect();
            check(RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                RUN_VALUE,
                REG_SZ.0,
                Some(value.as_ptr().cast()),
                (value.len() * 2) as u32,
            ))?;
            check(RegSetKeyValueW(
                HKEY_CURRENT_USER,
                APPROVED_KEY,
                RUN_VALUE,
                REG_BINARY.0,
                Some(APPROVED_ENABLED.as_ptr().cast()),
                APPROVED_ENABLED.len() as u32,
            ))
        } else {
            check(RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE))?;
            // Peut ne pas exister : seule l'absence compte.
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, APPROVED_KEY, RUN_VALUE);
            Ok(())
        }
    }
}

fn check(status: windows::Win32::Foundation::WIN32_ERROR) -> Result<()> {
    if status != ERROR_SUCCESS {
        bail!("registre (démarrage avec Windows) : erreur {}", status.0);
    }
    Ok(())
}

/// Commande de démarrage : chemin entre guillemets (les espaces sont courants).
fn run_command(exe: &str) -> String {
    format!("\"{exe}\"")
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
    fn startup_approval_follows_task_manager_state() {
        assert!(is_approved(Some(&APPROVED_ENABLED)));
        assert!(is_approved(Some(&[6, 0, 0, 0])));
        assert!(!is_approved(Some(&[3, 0, 0, 0])));
        // Absente : Windows ne lance pas l'entrée.
        assert!(!is_approved(None));
        assert!(!is_approved(Some(&[])));
    }

    #[test]
    fn run_command_quotes_the_path() {
        assert_eq!(
            run_command(r"C:\Users\Jo Doe\clipper.exe"),
            r#""C:\Users\Jo Doe\clipper.exe""#
        );
    }
}
