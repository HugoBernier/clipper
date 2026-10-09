//! Spike C : fenêtre WebView2. `cargo run --example webview [-- <dossier des clips>]`
//!
//! Vérifie, avant l'itération 9 : une fenêtre Win32 qui affiche une page HTML, la
//! lecture d'un clip dans un `<video>` (dossier mappé sur un nom d'hôte virtuel), un
//! aller-retour page → Rust → page (liste, copie dans le presse-papiers), le
//! glisser-déposer d'un clip vers Discord, et la mémoire rendue à la fermeture
//! (processus `msedgewebview2.exe` comptés avant, à la fermeture et 3 s après).

use std::cell::RefCell;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    NavigationStartingEventHandler, WebMessageReceivedEventHandler, take_pwstr,
};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::Com::{CoTaskMemFree, IDataObject};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::{
    DROPEFFECT_COPY, IDropSource, OleFlushClipboard, OleInitialize, OleSetClipboard,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_LBUTTON};
use windows::Win32::UI::Shell::{
    BHID_DataObject, FOLDERID_Videos, IShellItem, KF_FLAG_DEFAULT, SHCreateItemFromParsingName,
    SHDoDragDrop, SHGetKnownFolderPath,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, Interface, PCWSTR, PWSTR, w};

const HOST: &str = "clips.clipper";
/// Glisser demandé par la page, lancé depuis la boucle de messages.
const WM_DRAG: u32 = WM_APP + 1;

const PAGE: &str = r#"<!doctype html><meta charset=utf-8><title>Clipper</title>
<style>
body{font:14px system-ui;margin:0;display:flex;height:100vh}
#list{width:300px;overflow:auto;border-right:1px solid #8884}
#list div{padding:6px 8px;cursor:pointer;user-select:none}
#list div.on{background:#8883}
main{flex:1;display:flex;flex-direction:column;gap:8px;padding:8px;min-width:0}
video{flex:1;min-height:0;width:100%;background:#000}
</style>
<div id=list></div>
<main><video id=player controls></video>
<div><button id=copy disabled>Copier</button> <span id=status></span></div></main>
<script>
const host = window.chrome.webview;
let current = null;
host.addEventListener('message', e => {
  const [kind, ...rest] = e.data.split('\n');
  if (kind === 'list') render(rest.filter(Boolean)); else status.textContent = rest.join(' ');
});
function render(names) {
  list.textContent = '';
  for (const name of names) {
    const row = document.createElement('div');
    row.textContent = name;
    row.onclick = () => select(name, row);
    // Glisser : le drag HTML ne transporte pas de fichier, Rust lance un glisser natif.
    row.onpointerdown = down => {
      if (down.button !== 0) return;
      const move = e => {
        if (Math.abs(e.clientX - down.clientX) + Math.abs(e.clientY - down.clientY) < 6) return;
        stop();
        host.postMessage('drag\n' + name);
      };
      const stop = () => {
        removeEventListener('pointermove', move);
        removeEventListener('pointerup', stop);
        removeEventListener('pointercancel', stop);
      };
      addEventListener('pointermove', move);
      addEventListener('pointerup', stop);
      addEventListener('pointercancel', stop);
    };
    list.append(row);
  }
}
function select(name, row) {
  current = name;
  for (const r of list.children) r.classList.toggle('on', r === row);
  player.src = 'https://clips.clipper/' + encodeURIComponent(name);
  player.play().catch(() => {});
  copy.disabled = false;
}
copy.onclick = () => host.postMessage('copy\n' + current);
host.postMessage('list');
</script>"#;

thread_local! {
    static CONTROLLER: RefCell<Option<ICoreWebView2Controller>> = const { RefCell::new(None) };
    static PENDING_DRAG: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

fn main() -> Result<()> {
    let dir = match std::env::args().nth(1) {
        Some(dir) => PathBuf::from(dir),
        None => videos_dir()?.join("Clipper"),
    };
    // Le mappage d'hôte virtuel et le Shell veulent un chemin absolu.
    let dir = std::fs::canonicalize(&dir).with_context(|| format!("{}", dir.display()))?;
    println!("clips : {}", dir.display());
    // Mêmes conditions que Clipper (main.rs) : coordonnées du glisser non virtualisées.
    // SAFETY: réglage du processus, avant toute création de fenêtre.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
        .context("SetProcessDpiAwarenessContext")?;
    // SAFETY: init OLE (STA, exigé par WebView2, le presse-papiers et le glisser) une fois.
    unsafe { OleInitialize(None).context("OleInitialize")? };
    register_class()?;
    println!("avant       : {}", webview_processes());
    loop {
        let start = Instant::now();
        open_window(&dir)?;
        println!("fermée après {:.0} s", start.elapsed().as_secs_f64());
        pump_for(Duration::from_secs(3));
        println!("après 3 s   : {}", webview_processes());
        // SAFETY: boîte modale sans fenêtre parente.
        let again = unsafe {
            MessageBoxW(
                None,
                w!("Rouvrir la fenêtre ?"),
                w!("Spike WebView2"),
                MB_YESNO | MB_ICONQUESTION,
            )
        };
        if again != IDYES {
            return Ok(());
        }
    }
}

fn register_class() -> Result<()> {
    // SAFETY: classe enregistrée une fois, avec une procédure `extern "system"` valide.
    unsafe {
        let instance = GetModuleHandleW(None).context("GetModuleHandleW")?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: HINSTANCE(instance.0),
            hCursor: LoadCursorW(None, IDC_ARROW).context("LoadCursorW")?,
            lpszClassName: w!("ClipperSpikeWebView"),
            ..Default::default()
        };
        ensure!(RegisterClassW(&class) != 0, "RegisterClassW");
    }
    Ok(())
}

/// Ouvre la fenêtre et rend la main quand elle est détruite, moteur libéré.
fn open_window(dir: &Path) -> Result<()> {
    // SAFETY: classe enregistrée par `register_class` ; fenêtre utilisée sur ce thread.
    let hwnd = unsafe {
        CreateWindowExW(
            Default::default(),
            w!("ClipperSpikeWebView"),
            w!("Clipper (spike WebView2)"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1100,
            700,
            None,
            None,
            None,
            None,
        )
    }
    .context("CreateWindowExW")?;

    let opened = Instant::now();
    let environment = create_environment()?;
    let controller = create_controller(&environment, hwnd)?;
    println!("moteur prêt en {} ms", opened.elapsed().as_millis());
    // SAFETY: contrôleur vivant, sur le thread qui l'a créé.
    let webview = unsafe { controller.CoreWebView2() }.context("CoreWebView2")?;
    let mapping = webview
        .cast::<ICoreWebView2_3>()
        .context("ICoreWebView2_3 (runtime trop ancien)")?;
    // SAFETY: chaînes valides pendant l'appel.
    unsafe {
        mapping.SetVirtualHostNameToFolderMapping(
            &HSTRING::from(HOST),
            &HSTRING::from(dir.as_os_str()),
            COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_DENY_CORS,
        )
    }
    .context("SetVirtualHostNameToFolderMapping")?;

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
    let dir = dir.to_path_buf();
    let messages = WebMessageReceivedEventHandler::create(Box::new(move |sender, args| {
        let (Some(webview), Some(args)) = (sender, args) else {
            return Ok(());
        };
        let mut message = PWSTR::null();
        // SAFETY: arguments valides pendant l'événement ; chaîne libérée par take_pwstr.
        unsafe { args.TryGetWebMessageAsString(&mut message) }?;
        let reply = handle(&dir, hwnd, &take_pwstr(message))
            .unwrap_or_else(|e| format!("status\nerreur : {e:#}"));
        if !reply.is_empty() {
            // SAFETY: vue vivante, sur ce thread.
            unsafe { webview.PostWebMessageAsString(&HSTRING::from(reply)) }?;
        }
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

    CONTROLLER.with_borrow_mut(|c| *c = Some(controller));
    fit(hwnd);

    let mut msg = MSG::default();
    // SAFETY: boucle de messages classique, sur le thread de la fenêtre.
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    // `webview` et `environment` sont relâchés ici : plus aucune référence au moteur.
    Ok(())
}

/// Commandes de la page : `list`, `copy\n<nom>`, `drag\n<nom>`. Renvoie la réponse.
fn handle(dir: &Path, hwnd: HWND, message: &str) -> Result<String> {
    let (command, name) = message.split_once('\n').unwrap_or((message, ""));
    match command {
        "list" => Ok(format!("list\n{}", list_clips(dir)?.join("\n"))),
        "copy" => {
            let data = data_object(&clip_path(dir, name)?)?;
            // SAFETY: objet du shell ; le flush le rend au presse-papiers, qui ne dépend
            // plus de nous (le clip reste collable après la fermeture de Clipper).
            unsafe {
                OleSetClipboard(&data).context("OleSetClipboard")?;
                OleFlushClipboard().context("OleFlushClipboard")?;
            }
            Ok(format!("status\ncopié : {name}"))
        }
        "drag" => {
            // Le glisser est une boucle modale : pas dans un gestionnaire WebView2.
            let path = clip_path(dir, name)?;
            PENDING_DRAG.with_borrow_mut(|p| *p = Some(path));
            // SAFETY: fenêtre vivante, sur ce thread.
            unsafe { PostMessageW(Some(hwnd), WM_DRAG, WPARAM(0), LPARAM(0)) }
                .context("PostMessageW")?;
            Ok(String::new())
        }
        _ => Ok(format!("status\ncommande inconnue : {command}")),
    }
}

fn list_clips(dir: &Path) -> Result<Vec<String>> {
    let mut clips = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("lecture de {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension() == Some(OsStr::new("mp4")) {
            clips.push((entry.metadata()?.modified()?, entry.file_name()));
        }
    }
    clips.sort_by_key(|c| std::cmp::Reverse(c.0));
    Ok(clips
        .into_iter()
        .map(|(_, name)| name.to_string_lossy().into_owned())
        .collect())
}

/// Un nom venu de la page ne doit désigner qu'un clip du dossier (pas de `..`, de
/// séparateur, ni de flux NTFS `clip.mp4:x`).
fn clip_path(dir: &Path, name: &str) -> Result<PathBuf> {
    ensure!(
        Path::new(name).file_name() == Some(OsStr::new(name))
            && !name.contains(':')
            && Path::new(name).extension() == Some(OsStr::new("mp4")),
        "nom de clip invalide : {name}"
    );
    let path = dir.join(name);
    ensure!(path.is_file(), "clip introuvable : {name}");
    Ok(path)
}

/// Le même objet que l'Explorateur fournit pour un Ctrl+C ou un glisser.
fn data_object(path: &Path) -> Result<IDataObject> {
    // SAFETY: appels shell sur un chemin existant.
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None)
            .context("SHCreateItemFromParsingName")?;
        item.BindToHandler(None, &BHID_DataObject)
            .context("BindToHandler(DataObject)")
    }
}

fn create_environment() -> Result<ICoreWebView2Environment> {
    // Profil distinct de celui de Clipper : ne pas rejoindre son moteur (mesures faussées).
    let data = std::env::var("LOCALAPPDATA").context("LOCALAPPDATA")? + r"\clipper\webview2-spike";
    let (tx, rx) = std::sync::mpsc::channel();
    CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| {
            // SAFETY: chaînes valides pendant l'appel ; options par défaut.
            unsafe {
                CreateCoreWebView2EnvironmentWithOptions(
                    PCWSTR::null(),
                    &HSTRING::from(data),
                    None::<&ICoreWebView2EnvironmentOptions>,
                    &handler,
                )
            }
            .map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(move |result, environment| {
            result?;
            let _ = tx.send(environment);
            Ok(())
        }),
    )
    .context("création de l'environnement WebView2 (runtime installé ?)")?;
    rx.recv()?.context("environnement WebView2 absent")
}

fn create_controller(
    environment: &ICoreWebView2Environment,
    hwnd: HWND,
) -> Result<ICoreWebView2Controller> {
    let environment = environment.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| {
            // SAFETY: fenêtre parente vivante, sur ce thread.
            unsafe { environment.CreateCoreWebView2Controller(hwnd, &handler) }
                .map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(move |result, controller| {
            result?;
            let _ = tx.send(controller);
            Ok(())
        }),
    )
    .context("création du contrôleur WebView2")?;
    rx.recv()?.context("contrôleur WebView2 absent")
}

fn fit(hwnd: HWND) {
    CONTROLLER.with_borrow(|controller| {
        let Some(controller) = controller else { return };
        let mut rect = RECT::default();
        // SAFETY: fenêtre et contrôleur vivants, sur ce thread.
        let result = unsafe { GetClientRect(hwnd, &mut rect) }
            .context("GetClientRect")
            .and_then(|()| unsafe { controller.SetBounds(rect) }.context("SetBounds"));
        if let Err(e) = result {
            eprintln!("redimensionnement : {e:#}");
        }
    });
}

fn notify_moved() {
    CONTROLLER.with_borrow(|controller| {
        if let Some(controller) = controller {
            // SAFETY: contrôleur vivant, sur ce thread.
            if let Err(e) = unsafe { controller.NotifyParentWindowPositionChanged() } {
                eprintln!("NotifyParentWindowPositionChanged : {e:#}");
            }
        }
    });
}

fn focus() {
    CONTROLLER.with_borrow(|controller| {
        if let Some(controller) = controller {
            // SAFETY: contrôleur vivant, sur ce thread.
            if let Err(e) =
                unsafe { controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC) }
            {
                eprintln!("MoveFocus : {e:#}");
            }
        }
    });
}

fn drag(hwnd: HWND) {
    let Some(path) = PENDING_DRAG.with_borrow_mut(Option::take) else {
        return;
    };
    // Demande asynchrone : le bouton a pu être relâché entre-temps (le shell déposerait
    // aussitôt sous le curseur).
    // SAFETY: GetKeyState n'a pas de précondition.
    if unsafe { GetKeyState(i32::from(VK_LBUTTON.0)) } >= 0 {
        println!("glisser annulé : bouton déjà relâché");
        return;
    }
    let result = data_object(&path).and_then(|data| {
        // SAFETY: boucle modale du shell, lancée hors de tout gestionnaire WebView2 ;
        // sans IDropSource, le shell fournit le sien (curseurs, Échap pour annuler).
        unsafe { SHDoDragDrop(Some(hwnd), &data, None::<&IDropSource>, DROPEFFECT_COPY) }
            .context("SHDoDragDrop")
    });
    println!("glisser {} : {result:?}", path.display());
}

extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_SIZE => fit(hwnd),
        // Sans ça, les listes déroulantes s'ouvrent à l'ancienne position de la fenêtre,
        // et le clavier n'atteint pas la page avant un clic.
        WM_MOVE => notify_moved(),
        WM_SETFOCUS => focus(),
        WM_DRAG => drag(hwnd),
        WM_CLOSE => {
            println!("pendant     : {}", webview_processes());
            if let Some(controller) = CONTROLLER.with_borrow_mut(Option::take) {
                // SAFETY: contrôleur vivant ; Close libère la vue avant la fenêtre.
                if let Err(e) = unsafe { controller.Close() } {
                    eprintln!("Close : {e:#}");
                }
            }
            // SAFETY: notre fenêtre, sur ce thread.
            if let Err(e) = unsafe { DestroyWindow(hwnd) } {
                eprintln!("DestroyWindow : {e:#}");
            }
        }
        // SAFETY: termine la boucle de messages de `open_window`.
        WM_DESTROY => unsafe { PostQuitMessage(0) },
        // SAFETY: traitement par défaut des autres messages.
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

/// Laisse passer les messages (libérations asynchrones de WebView2) pendant `duration`.
fn pump_for(duration: Duration) {
    let end = Instant::now() + duration;
    let mut msg = MSG::default();
    while Instant::now() < end {
        // SAFETY: messages de ce thread, distribués normalement.
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Processus `msedgewebview2.exe` lancés par nous (navigateur et ses enfants) et notre RAM.
fn webview_processes() -> String {
    let script = format!(
        "$all = Get-CimInstance Win32_Process -Filter \"Name='msedgewebview2.exe'\"; \
         $b = @($all | ? {{ $_.ParentProcessId -eq {pid} }}); \
         $t = $b + @($all | ? {{ $b.ProcessId -contains $_.ParentProcessId }}); \
         $me = (Get-Process -Id {pid}).WorkingSet64; \
         '{{0}} processus WebView2, {{1:N0}} Mo ; Clipper {{2:N0}} Mo' -f $t.Count, \
         (($t | Measure-Object WorkingSetSize -Sum).Sum / 1MB), ($me / 1MB)",
        pid = std::process::id()
    );
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|e| format!("mesure impossible : {e}"))
}

fn videos_dir() -> Result<PathBuf> {
    // SAFETY: chaîne allouée par le shell, copiée puis libérée par CoTaskMemFree.
    unsafe {
        let path = SHGetKnownFolderPath(&FOLDERID_Videos, KF_FLAG_DEFAULT, None)
            .context("dossier Vidéos")?;
        let result = path.to_string();
        CoTaskMemFree(Some(path.0 as _));
        Ok(PathBuf::from(result?))
    }
}
