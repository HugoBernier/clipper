//! Fenêtre de Clipper : une fenêtre Win32 dont le contenu (`ui.html`) est affiché par
//! WebView2, le moteur d'Edge présent dans Windows. Le moteur n'existe que pendant que
//! la fenêtre est ouverte : la fermer le libère.
//!
//! Tout se passe sur le thread principal. La page envoie des messages texte, mis en file
//! puis signalés à la fenêtre principale par `WM_UI` ; `main` les traite et répond par
//! `post`.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Once;

use anyhow::{Context, Result};
use log::{error, info};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2, ICoreWebView2Controller,
    ICoreWebView2Environment, ICoreWebView2EnvironmentOptions,
};
use webview2_com::{
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    NavigationStartingEventHandler, WebMessageReceivedEventHandler, take_pwstr,
};
use windows::Win32::Foundation::{ERROR_CANCELLED, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Shell::{
    FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem,
    SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HRESULT, HSTRING, PCWSTR, PWSTR, w};

use crate::tray;

/// Envoyé à la fenêtre principale : des messages de la page attendent (`take_requests`).
pub const WM_UI: u32 = WM_APP + 3;
const CLASS: PCWSTR = w!("ClipperWindow");
const PAGE: &str = include_str!("ui.html");
/// Taille à 100 % d'échelle (96 dpi).
const WIDTH: i32 = 640;
const HEIGHT: i32 = 720;

struct Window {
    hwnd: HWND,
    /// Absent tant que le moteur démarre (création asynchrone).
    controller: Option<ICoreWebView2Controller>,
    webview: Option<ICoreWebView2>,
}

thread_local! {
    static WINDOW: RefCell<Option<Window>> = const { RefCell::new(None) };
    static REQUESTS: RefCell<VecDeque<String>> = const { RefCell::new(VecDeque::new()) };
}

/// Ouvre la fenêtre, ou la ramène au premier plan si elle l'est déjà.
pub fn open() -> Result<()> {
    if let Some(hwnd) = current_hwnd() {
        // SAFETY: notre fenêtre, sur ce thread.
        unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
        return Ok(());
    }
    register_class()?;
    // SAFETY: GetDpiForSystem n'a pas de précondition.
    let dpi = unsafe { GetDpiForSystem() } as i32;
    // SAFETY: classe enregistrée ci-dessus ; fenêtre utilisée sur ce thread.
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CLASS,
            w!("Clipper"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            WIDTH * dpi / 96,
            HEIGHT * dpi / 96,
            None,
            None,
            None,
            None,
        )
    }
    .context("CreateWindowExW (fenêtre)")?;
    WINDOW.with_borrow_mut(|w| {
        *w = Some(Window {
            hwnd,
            controller: None,
            webview: None,
        })
    });
    // Création asynchrone, sans boucle de messages imbriquée : un appui sur le
    // raccourci pendant le démarrage du moteur n'est pas perdu.
    let data = HSTRING::from(user_data_dir()?.as_os_str());
    let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
        move |result, environment| {
            if let Err(e) = result
                .context("environnement WebView2")
                .and_then(|()| environment.context("environnement WebView2 absent"))
                .and_then(|environment| create_controller(hwnd, &environment))
            {
                fail(hwnd, &e);
            }
            Ok(())
        },
    ));
    // SAFETY: chaînes valides pendant l'appel ; le gestionnaire est gardé par WebView2.
    unsafe {
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            &data,
            None::<&ICoreWebView2EnvironmentOptions>,
            &handler,
        )
    }
    .context("WebView2 indisponible (runtime absent ?)")
    .inspect_err(|e| fail(hwnd, e))
}

/// Envoie un message à la page, si elle est chargée.
pub fn post(message: &str) {
    let Some(webview) = WINDOW.with_borrow(|w| w.as_ref().and_then(|w| w.webview.clone())) else {
        return;
    };
    // SAFETY: vue vivante, sur ce thread.
    if let Err(e) = unsafe { webview.PostWebMessageAsString(&HSTRING::from(message)) } {
        error!("message à la fenêtre : {e:#}");
    }
}

/// Fait relire l'état par la page ouverte (réglage changé depuis le menu de l'icône).
pub fn refresh() {
    if current_hwnd().is_some() {
        request("get");
    }
}

fn request(message: &str) {
    REQUESTS.with_borrow_mut(|r| r.push_back(message.into()));
    crate::post_to_main(WM_UI);
}

/// Messages de la page reçus depuis le dernier appel.
pub fn take_requests() -> Vec<String> {
    REQUESTS.with_borrow_mut(|r| r.drain(..).collect())
}

/// Sélecteur de dossier de Windows, ouvert sur `current`. `None` si annulé.
pub fn pick_folder(current: &Path) -> Result<Option<PathBuf>> {
    // SAFETY: objets COM du Shell, créés et utilisés sur ce thread (STA) ; la chaîne
    // du chemin est copiée puis libérée par CoTaskMemFree.
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
            .context("FileOpenDialog")?;
        let options = dialog.GetOptions().context("GetOptions")?;
        dialog
            .SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)
            .context("SetOptions")?;
        if let Ok(folder) = SHCreateItemFromParsingName::<_, _, IShellItem>(
            &HSTRING::from(current.as_os_str()),
            None,
        ) {
            dialog.SetFolder(&folder).context("SetFolder")?;
        }
        match dialog.Show(current_hwnd()) {
            Err(e) if e.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => return Ok(None),
            result => result.context("sélecteur de dossier")?,
        }
        let path = dialog
            .GetResult()
            .context("GetResult")?
            .GetDisplayName(SIGDN_FILESYSPATH)
            .context("GetDisplayName")?;
        let result = path.to_string();
        CoTaskMemFree(Some(path.0 as _));
        Ok(Some(PathBuf::from(result?)))
    }
}

fn current_hwnd() -> Option<HWND> {
    WINDOW.with_borrow(|w| w.as_ref().map(|w| w.hwnd))
}

fn register_class() -> Result<()> {
    static REGISTERED: Once = Once::new();
    let mut result = Ok(());
    REGISTERED.call_once(|| {
        // SAFETY: classe enregistrée une fois, procédure `extern "system"` valide.
        result = unsafe {
            GetModuleHandleW(None).and_then(|instance| {
                let class = WNDCLASSW {
                    lpfnWndProc: Some(window_proc),
                    hInstance: instance.into(),
                    hCursor: LoadCursorW(None, IDC_ARROW)?,
                    // SAFETY (MAKEINTRESOURCE) : id de ressource casté en pointeur.
                    hIcon: LoadIconW(
                        Some(instance.into()),
                        PCWSTR(tray::ICON_RESOURCE as usize as *const u16),
                    )?,
                    lpszClassName: CLASS,
                    ..Default::default()
                };
                if RegisterClassW(&class) == 0 {
                    return Err(windows::core::Error::from_thread());
                }
                Ok(())
            })
        }
        .context("RegisterClassW (fenêtre)");
    });
    result
}

/// `%LOCALAPPDATA%\clipper\webview2` : profil du moteur (cache), à côté du log.
fn user_data_dir() -> Result<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA absent")?;
    Ok(PathBuf::from(base).join("clipper").join("webview2"))
}

fn create_controller(hwnd: HWND, environment: &ICoreWebView2Environment) -> Result<()> {
    let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
        move |result, controller| {
            if let Err(e) = result
                .context("contrôleur WebView2")
                .and_then(|()| controller.context("contrôleur WebView2 absent"))
                .and_then(|controller| attach(hwnd, controller))
            {
                fail(hwnd, &e);
            }
            Ok(())
        },
    ));
    // SAFETY: fenêtre parente vivante, sur ce thread.
    unsafe { environment.CreateCoreWebView2Controller(hwnd, &handler) }
        .context("CreateCoreWebView2Controller")
}

/// Branche la vue prête sur la fenêtre et charge la page.
fn attach(hwnd: HWND, controller: ICoreWebView2Controller) -> Result<()> {
    // Fenêtre fermée pendant le démarrage du moteur : on libère la vue aussitôt.
    if current_hwnd() != Some(hwnd) {
        // SAFETY: contrôleur vivant, sur ce thread.
        unsafe { controller.Close() }.context("Close")?;
        return Ok(());
    }
    // SAFETY: contrôleur vivant, sur ce thread.
    let webview = unsafe { controller.CoreWebView2() }.context("CoreWebView2")?;
    // Un fichier déposé sur la fenêtre, ou un lien, ferait naviguer la vue : la page
    // remplaçante pourrait alors envoyer des commandes. Seule notre page est chargée.
    let mut first = true;
    let guard = NavigationStartingEventHandler::create(Box::new(move |_, args| {
        if let Some(args) = args.filter(|_| !first) {
            // SAFETY: arguments valides pendant l'événement.
            unsafe { args.SetCancel(true) }?;
        }
        first = false;
        Ok(())
    }));
    let messages = WebMessageReceivedEventHandler::create(Box::new(|_, args| {
        let Some(args) = args else {
            return Ok(());
        };
        let mut message = PWSTR::null();
        // SAFETY: arguments valides pendant l'événement ; chaîne libérée par take_pwstr.
        unsafe { args.TryGetWebMessageAsString(&mut message) }?;
        // Traité par la fenêtre principale, hors du gestionnaire de WebView2 (un
        // sélecteur de dossier y serait une boucle modale imbriquée).
        request(&take_pwstr(message));
        Ok(())
    }));
    let mut token = 0;
    // SAFETY: gestionnaires COM valides, vue vivante ; ils partent avec le contrôleur.
    unsafe {
        webview
            .add_NavigationStarting(&guard, &mut token)
            .context("add_NavigationStarting")?;
        webview
            .add_WebMessageReceived(&messages, &mut token)
            .context("add_WebMessageReceived")?;
        webview
            .NavigateToString(&HSTRING::from(PAGE))
            .context("NavigateToString")?;
    }
    WINDOW.with_borrow_mut(|w| {
        if let Some(w) = w {
            w.controller = Some(controller);
            w.webview = Some(webview);
        }
    });
    fit(hwnd);
    info!("fenêtre ouverte");
    Ok(())
}

/// Le moteur n'a pas démarré : on le dit, et on ferme la fenêtre vide.
fn fail(hwnd: HWND, e: &anyhow::Error) {
    error!("fenêtre : {e:#}");
    // SAFETY: fenêtre vivante (sinon l'appel échoue sans effet), sur ce thread.
    unsafe {
        let _ = MessageBoxW(
            Some(hwnd),
            &HSTRING::from(format!(
                "La fenêtre de Clipper n'a pas pu s'ouvrir.\n\n{e:#}"
            )),
            w!("Clipper"),
            MB_OK | MB_ICONERROR,
        );
        let _ = DestroyWindow(hwnd);
    }
}

fn fit(hwnd: HWND) {
    WINDOW.with_borrow(|w| {
        let Some(controller) = w.as_ref().and_then(|w| w.controller.as_ref()) else {
            return;
        };
        let mut rect = RECT::default();
        // SAFETY: fenêtre et contrôleur vivants, sur ce thread.
        let result = unsafe { GetClientRect(hwnd, &mut rect) }
            .context("GetClientRect")
            .and_then(|()| unsafe { controller.SetBounds(rect) }.context("SetBounds"));
        if let Err(e) = result {
            error!("redimensionnement : {e:#}");
        }
    });
}

extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_SIZE => fit(hwnd),
        WM_DPICHANGED => {
            // Nouvel écran ou nouvelle échelle : Windows propose la taille à adopter.
            // SAFETY: pour WM_DPICHANGED, lParam pointe sur un RECT valide.
            let rect = unsafe { *(lparam.0 as *const RECT) };
            // SAFETY: notre fenêtre, sur ce thread.
            let _ = unsafe {
                SetWindowPos(
                    hwnd,
                    None,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
        }
        WM_CLOSE => {
            let controller =
                WINDOW.with_borrow_mut(|w| w.as_mut().and_then(|w| w.controller.take()));
            if let Some(controller) = controller {
                // SAFETY: contrôleur vivant ; Close libère le moteur avant la fenêtre.
                if let Err(e) = unsafe { controller.Close() } {
                    error!("fermeture de la vue : {e:#}");
                }
            }
            // SAFETY: notre fenêtre, sur ce thread.
            if let Err(e) = unsafe { DestroyWindow(hwnd) } {
                error!("DestroyWindow : {e:#}");
            }
        }
        // Pas de PostQuitMessage : Clipper continue dans la zone de notification.
        WM_DESTROY => {
            WINDOW.with_borrow_mut(|w| *w = None);
            // Une capture du raccourci en cours prend fin.
            request("closed");
            info!("fenêtre fermée");
        }
        // SAFETY: traitement par défaut des autres messages.
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}
