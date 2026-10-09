#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod config;
mod library;
mod mf;
mod mix;
mod mp4box;
mod ring;
mod save;
mod shell;
mod tray;
mod ui;
mod video;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use log::{error, info, warn};
use windows::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, ERROR_HOTKEY_ALREADY_REGISTERED, ERROR_SHARING_VIOLATION, GetLastError,
    HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::OleInitialize;
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentThreadId};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey,
};
use windows::Win32::UI::Shell::COPYENGINE_E_SHARING_VIOLATION_SRC;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HWND_MESSAGE, MB_ICONHAND,
    MB_OK, MB_SETFOREGROUND, MB_TOPMOST, MESSAGEBOX_STYLE, MSG, MessageBoxW, PostMessageW,
    PostQuitMessage, RegisterClassW, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP,
    WM_HOTKEY, WNDCLASSW,
};
use windows::core::{HRESULT, HSTRING, w};

use crate::audio::Microphone;
use crate::config::{Config, Hotkey, parse_hotkey};
use crate::mf::{AudioFormat, SEC, VideoFormat};
use crate::ring::Ring;
use crate::video::Video;

/// AAC 160 kb/s (débits possibles de l'encodeur Windows : 96, 128, 160, 192).
const AUDIO_BPS: u32 = 160_000;
/// La liste des clips a changé (clip sauvegardé, autre dossier).
const WM_CLIPS_CHANGED: u32 = WM_APP + 4;
/// Identifiant du raccourci global (un seul).
const HOTKEY_ID: i32 = 1;
/// Délai max d'une sauvegarde en cours, avant de quitter ou de changer de format.
const SAVE_WAIT: Duration = Duration::from_secs(10);
/// Délai max pour que l'encodeur rattrape l'instant de l'appui.
const CATCH_UP: Duration = Duration::from_secs(2);

fn main() -> Result<()> {
    if already_running() {
        // Lancé deux fois (tâche de démarrage + menu Démarrer) : on laisse la première
        // instance tranquille, sans toucher à son log.
        return Ok(());
    }
    init_log()?;
    // panic = "abort" en release : le hook est le seul endroit où la consigner.
    std::panic::set_hook(Box::new(|info| {
        error!("panic : {info}");
        report_stop(&format!("panic : {info}"));
    }));
    if let Err(e) = run() {
        error!("{e:#}");
        report_stop(&format!("{e:#}"));
        return Err(e);
    }
    Ok(())
}

/// Clipper s'arrête : sans fenêtre ni icône, un son passerait inaperçu, on l'écrit. Au
/// premier plan, pour ne pas s'ouvrir derrière un jeu en plein écran.
fn report_stop(cause: &str) {
    let text = format!(
        "Clipper s'est arrêté :\n\n{cause}\n\nDétails dans %LOCALAPPDATA%\\clipper\\{}",
        log_name()
    );
    // SAFETY: boîte modale sans fenêtre parente, chaînes valides pendant l'appel.
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(text),
            w!("Clipper"),
            MB_OK | MB_ICONHAND | MB_SETFOREGROUND | MB_TOPMOST,
        )
    };
}

/// Nom distinct en debug : ne pas écraser le log de la version installée.
fn log_name() -> &'static str {
    if cfg!(debug_assertions) {
        "clipper-debug.log"
    } else {
        "clipper.log"
    }
}

/// Log dans `%LOCALAPPDATA%\clipper\clipper.log`, recréé à chaque démarrage (pas de
/// console en release). En debug, aussi sur la sortie d'erreur.
/// `CLIPPER_DEBUG=1` : logs détaillés (diagnostic).
fn init_log() -> Result<()> {
    let dir = PathBuf::from(std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA absent")?)
        .join("clipper");
    std::fs::create_dir_all(&dir)?;
    let file = std::fs::File::create(dir.join(log_name()))?;
    let level = if std::env::var_os("CLIPPER_DEBUG").is_some() {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };
    let mut config = simplelog::ConfigBuilder::new();
    // Appelé avant tout autre thread ; sans effet de bord s'il échoue (reste en UTC).
    let _ = config.set_time_offset_to_local();
    let config = config.build();
    let mut loggers: Vec<Box<dyn simplelog::SharedLogger>> =
        vec![simplelog::WriteLogger::new(level, config.clone(), file)];
    if cfg!(debug_assertions) {
        loggers.push(simplelog::SimpleLogger::new(level, config));
    }
    simplelog::CombinedLogger::init(loggers)?;
    Ok(())
}

/// Instance unique par session (mutex nommé, gardé jusqu'à la fin du processus). La
/// build debug a son propre nom pour tourner à côté de la version installée.
fn already_running() -> bool {
    let name = if cfg!(debug_assertions) {
        windows::core::w!(r"Local\ClipperDebug")
    } else {
        windows::core::w!(r"Local\Clipper")
    };
    // SAFETY: création d'un mutex nommé ; le handle est volontairement gardé ouvert.
    unsafe {
        let created = CreateMutexW(None, false, name);
        created.is_ok() && GetLastError() == ERROR_ALREADY_EXISTS
    }
}

/// Seul retour sans interface : un son de succès, un son d'erreur.
fn beep(ok: bool) {
    let sound: MESSAGEBOX_STYLE = if ok { MB_OK } else { MB_ICONHAND };
    // SAFETY: MessageBeep n'a pas de précondition.
    let _ = unsafe { MessageBeep(sound) };
}

fn run() -> Result<()> {
    // Fenêtre et icône nettes à toutes les échelles d'affichage.
    // SAFETY: réglage du processus, avant toute création de fenêtre.
    if let Err(e) =
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
    {
        warn!("mise à l'échelle DPI : {e:#}");
    }
    // STA : exigé par WebView2 (fenêtre) et par les boîtes de dialogue du Shell.
    // SAFETY: initialisation OLE du thread principal, une fois.
    unsafe { OleInitialize(None) }.context("OleInitialize")?;
    let exe = std::env::current_exe()?;
    let exe_dir = exe.parent().context("dossier de l'exe")?;
    let config_path = exe_dir.join("clipper.toml");
    let config = Config::load_or_create(&config_path)?;
    let hotkey = parse_hotkey(&config.hotkey)?;
    let out_dir = save::videos_dir()?.join(&config.output_dir);

    let ring = Arc::new(Mutex::new(Ring::new(keep(&config))));
    let video = spawn_video(&config, &ring)?;
    let microphone = Arc::new(Microphone::new(
        config.microphone,
        config.microphone_volume,
        config.microphone_device.clone(),
    ));
    // Sans audio (aucune sortie son, encodeur absent), on garde au moins la vidéo.
    let audio = audio::spawn(AUDIO_BPS, microphone.clone(), ring.clone())
        .inspect_err(|e| warn!("clips sans son : {e:#}"))
        .ok();
    let window = create_main_window()?;
    register_hotkey(window, &hotkey)
        .with_context(|| format!("raccourci {} indisponible (déjà pris ?)", config.hotkey))?;
    info!(
        "prêt : {} sauvegarde les {} dernières secondes dans {}",
        config.hotkey,
        config.clip_seconds,
        out_dir.display()
    );
    // Sans icône (Explorateur absent…), Clipper reste utilisable au raccourci.
    let tray = tray::Tray::new(out_dir.clone(), tooltip(&config))
        .inspect_err(|e| warn!("icône de notification indisponible : {e:#}"))
        .ok();
    tray::set_microphone(config.microphone);
    ui::set_clips_dir(&out_dir);
    let saving = Arc::new(AtomicBool::new(false));
    APP.with_borrow_mut(|app| {
        *app = Some(App {
            config,
            config_path,
            out_dir,
            ring,
            video,
            audio,
            microphone,
            tray,
            window,
            hotkey_paused: false,
            saving: saving.clone(),
        })
    });

    let mut msg = MSG::default();
    // SAFETY: boucle de messages standard du thread courant.
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        // Tout passe par des fenêtres (voir `create_main_window`).
        // SAFETY: message reçu par GetMessageW, transmis tel quel.
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    // « Quitter » : on laisse finir une sauvegarde en cours. L'état est libéré avant
    // de rendre la main (icône retirée), y compris après une erreur vidéo.
    wait_for_save(&saving);
    APP.with_borrow_mut(|app| *app = None);
    // WM_QUIT avec wParam 1 : le thread vidéo s'est arrêté sur une erreur (déjà loguée).
    if msg.wParam.0 == 1 {
        let reason = video::stop_reason().unwrap_or_else(|| "cause inconnue".into());
        bail!("vidéo arrêtée : {reason}");
    }
    info!("arrêt demandé");
    Ok(())
}

/// Ce que les réglages changent à chaud, possédé par le thread principal.
struct App {
    config: Config,
    config_path: PathBuf,
    out_dir: PathBuf,
    ring: Arc<Mutex<Ring>>,
    video: Video,
    microphone: Arc<Microphone>,
    tray: Option<tray::Tray>,
    audio: Option<AudioFormat>,
    /// Reçoit le raccourci et les demandes de l'icône et de la fenêtre.
    window: HWND,
    /// Raccourci désenregistré pendant que la fenêtre en capture un nouveau.
    hotkey_paused: bool,
    /// Une sauvegarde est en cours.
    saving: Arc<AtomicBool>,
}

thread_local! {
    /// L'état du thread principal, atteint par la procédure de la fenêtre principale.
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Exécute `f` sur l'état. Une boucle modale ouverte pendant un emprunt (aucune ne
/// l'est : voir `handle_ui`) rendrait l'état indisponible ; l'appel est alors ignoré.
fn with_app(f: impl FnOnce(&mut App)) {
    APP.with(|app| match app.try_borrow_mut() {
        Ok(mut app) => {
            if let Some(app) = app.as_mut() {
                f(app);
            }
        }
        Err(_) => error!("état occupé : message ignoré"),
    });
}

/// Message de la fenêtre : la réponse est toujours l'état complet, avec l'erreur
/// éventuelle.
fn handle_ui(request: &str) {
    let (command, arg) = request.split_once('\n').unwrap_or((request, ""));
    if handle_library(command, arg) {
        return;
    }
    // Le sélecteur de dossier est une boucle modale : on l'ouvre sans tenir l'état,
    // pour qu'un appui sur le raccourci pendant ce temps soit traité.
    let mut picked = Ok(None);
    if command == "pick_folder" {
        let mut current = PathBuf::new();
        with_app(|app| current = app.out_dir.clone());
        picked = ui::pick_folder(&current);
    }
    with_app(|app| {
        let result = match command {
            // Sinon Windows prendrait le raccourci actuel avant la page.
            "capture" => app.pause_hotkey(),
            "get" | "closed" => Ok(()),
            "set" => match arg.split_once('=') {
                Some((key, value)) => app.change(key, value),
                None => Err(anyhow!("réglage mal formé : {arg}")),
            },
            "pick_folder" => match picked {
                Ok(Some(dir)) => app.change("output_dir", &dir.display().to_string()),
                Ok(None) => Ok(()),
                Err(e) => Err(e),
            },
            "startup" => tray::set_startup(arg == "true"),
            _ => Err(anyhow!("commande inconnue : {command}")),
        };
        // Toute autre demande clôt une capture du raccourci (Échap, fenêtre quittée…).
        let result = match command {
            "capture" => result,
            _ => result.and(app.resume_hotkey()),
        };
        match result {
            // La page attend la combinaison : un état la ferait sortir de la capture.
            Ok(()) if command == "capture" => {}
            // Fenêtre fermée : personne à qui répondre.
            Ok(()) if command == "closed" => {}
            Ok(()) => ui::post(&app.state()),
            Err(e) => {
                warn!("réglage refusé ({command} {arg}) : {e:#}");
                let message = setting_error(command, arg, &e);
                ui::post(&format!("{}error={message}\n", app.state()));
            }
        }
    });
}

/// Bibliothèque de clips ; `false` si `command` n'en relève pas. L'action est faite
/// sans tenir l'état (le glisser et la corbeille ouvrent des boucles modales), puis la
/// liste est renvoyée si elle a pu changer.
fn handle_library(command: &str, arg: &str) -> bool {
    let refresh = match command {
        "clips" | "delete" | "rename" => true,
        "copy" | "drag" | "reveal" | "open_folder" => false,
        _ => return false,
    };
    let mut dir = None;
    with_app(|app| dir = Some(app.out_dir.clone()));
    let Some(dir) = dir else {
        ui::post("error\nClipper est occupé, réessayez");
        return true;
    };
    let clip = |name: &str| -> Result<PathBuf> {
        let path = library::clip_path(&dir, name)?;
        ensure!(path.is_file(), "clip introuvable : {name}");
        Ok(path)
    };
    let owner = ui::current_hwnd();
    let result = match command {
        "copy" => clip(arg).and_then(|p| shell::copy(&p)),
        "drag" => clip(arg).and_then(|p| shell::drag(owner, &p)),
        "reveal" => clip(arg).and_then(|p| shell::reveal(&p)),
        "open_folder" => shell::open_folder(&dir),
        "delete" => clip(arg).and_then(|p| retry_while_in_use(|| shell::recycle(owner, &p))),
        "rename" => match arg.split_once('\t') {
            Some((old, new)) => {
                let mut renamed = String::new();
                retry_while_in_use(|| library::rename(&dir, old, new).map(|n| renamed = n))
                    // La page resélectionne le clip sous son nouveau nom.
                    .inspect(|()| ui::post(&format!("renamed\n{renamed}")))
            }
            None => Err(anyhow!("renommage mal formé : {arg}")),
        },
        _ => Ok(()),
    };
    match &result {
        Ok(()) if command == "copy" => ui::post(&format!("copied\n{arg}")),
        Ok(()) => {}
        Err(e) => {
            warn!("{command} {arg} : {e:#}");
            ui::post(&format!("error\n{}", library_error(command, e)));
        }
    }
    if refresh {
        post_clips(&dir);
    }
    true
}

/// Message court pour la fenêtre (règles de texte du design system) ; le détail va au
/// log.
fn library_error(command: &str, e: &anyhow::Error) -> String {
    match command {
        // Les refus de nom sont rédigés pour l'utilisateur (`library::rename`).
        "rename" => e.to_string(),
        "copy" => "Copie impossible".into(),
        "drag" => "Glisser impossible".into(),
        "delete" => "Suppression impossible".into(),
        "open_folder" => "Dossier inaccessible".into(),
        _ => "Clip introuvable".into(),
    }
}

/// Message court pour un réglage refusé ; le détail va au log.
fn setting_error(command: &str, arg: &str, e: &anyhow::Error) -> &'static str {
    let key = arg.split_once('=').map_or("", |(key, _)| key);
    match (command, key) {
        _ if e.to_string() == NOT_SAVED => NOT_SAVED,
        ("set", "hotkey") if hotkey_taken(e) => "Raccourci déjà utilisé",
        ("set", "hotkey") => "Raccourci refusé",
        ("pick_folder", _) | ("set", "output_dir") => "Dossier inaccessible",
        ("startup", _) => "Démarrage avec Windows non modifié",
        _ => "Réglage non appliqué",
    }
}

fn hotkey_taken(e: &anyhow::Error) -> bool {
    let taken = HRESULT::from_win32(ERROR_HOTKEY_ALREADY_REGISTERED.0);
    e.chain().any(|cause| {
        cause
            .downcast_ref::<windows::core::Error>()
            .is_some_and(|w| w.code() == taken)
    })
}

/// Le lecteur vient de lâcher le clip, mais le moteur de la page ferme le fichier un
/// peu après : un renommage ou une suppression immédiats peuvent le trouver ouvert.
/// Seule cette erreur est réessayée (pas un refus, ni une suppression annulée).
fn retry_while_in_use(mut action: impl FnMut() -> Result<()>) -> Result<()> {
    for _ in 0..4 {
        match action() {
            Err(e) if file_in_use(&e) => std::thread::sleep(Duration::from_millis(150)),
            result => return result,
        }
    }
    action()
}

fn file_in_use(e: &anyhow::Error) -> bool {
    let violation = ERROR_SHARING_VIOLATION.0;
    e.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .and_then(std::io::Error::raw_os_error)
            == Some(violation as i32)
            || cause
                .downcast_ref::<windows::core::Error>()
                .is_some_and(|w| {
                    w.code() == HRESULT::from_win32(violation)
                        || w.code() == COPYENGINE_E_SHARING_VIOLATION_SRC
                })
    })
}

fn post_clips(dir: &Path) {
    match library::list(dir) {
        Ok(clips) => {
            let lines: Vec<String> = clips.iter().map(library::ClipInfo::line).collect();
            ui::post(&format!("clips\n{}", lines.join("\n")));
        }
        Err(e) => {
            warn!("liste des clips : {e:#}");
            ui::post("error\nDossier illisible");
        }
    }
}

impl App {
    /// Appui sur le raccourci : sauvegarde dans un thread, une seule à la fois.
    fn save(&self) {
        let pressed = mf::now();
        // Comme OBS : une seule sauvegarde à la fois, l'appui suivant est ignoré.
        if self.saving.swap(true, Ordering::SeqCst) {
            warn!("sauvegarde déjà en cours, appui ignoré");
            return;
        }
        let clip = i64::from(self.config.clip_seconds) * SEC;
        let (ring, saving, out_dir, audio, format) = (
            self.ring.clone(),
            self.saving.clone(),
            self.out_dir.clone(),
            self.audio.clone(),
            self.video.format,
        );
        std::thread::spawn(move || {
            match save_clip(&ring, pressed, clip, &format, audio.as_ref(), &out_dir) {
                Ok(path) => {
                    info!("clip sauvegardé : {}", path.display());
                    beep(true);
                    // La bibliothèque ouverte affiche le nouveau clip.
                    post_to_main(WM_CLIPS_CHANGED);
                }
                Err(e) => {
                    error!("échec de la sauvegarde : {e:#}");
                    beep(false);
                }
            }
            saving.store(false, Ordering::SeqCst);
        });
    }

    /// Changement venu d'ailleurs que la fenêtre (menu de l'icône) : la fenêtre, si elle
    /// est ouverte, suit.
    fn change_and_report(&mut self, key: &str, value: &str) {
        // L'état envoyé à la page met fin à une capture du raccourci en cours.
        if let Err(e) = self.change(key, value).and(self.resume_hotkey()) {
            error!("réglage {key} : {e:#}");
            beep(false);
        }
        // L'état liste les micros (énumération de périphériques) : seulement si la
        // fenêtre est là pour le lire.
        if ui::current_hwnd().is_some() {
            ui::post(&self.state());
        }
    }

    /// Applique un réglage sans relancer Clipper, puis l'enregistre. Seul ce qui dépend
    /// du réglage redémarre : la vidéo pour la résolution, les fps ou la qualité ;
    /// l'audio n'est jamais interrompu.
    fn change(&mut self, key: &str, value: &str) -> Result<()> {
        let mut next = self.config.clone();
        next.set(key, value)?;
        let out_dir = save::videos_dir()?.join(&next.output_dir);
        if next.hotkey != self.config.hotkey {
            self.replace_hotkey(&next.hotkey)?;
        }
        if (next.height, next.fps, next.quality)
            != (self.config.height, self.config.fps, self.config.quality)
        {
            self.restart_video(&next)?;
        }
        self.ring
            .lock()
            .map_err(|_| anyhow!("ring empoisonné"))?
            .set_keep(keep(&next));
        self.microphone
            .enabled
            .store(next.microphone, Ordering::Relaxed);
        self.microphone
            .volume
            .store(next.microphone_volume, Ordering::Relaxed);
        self.microphone.set_device(next.microphone_device.clone());
        tray::set_microphone(next.microphone);
        // Remapper le dossier du lecteur couperait une lecture en cours : seulement s'il
        // change, et la liste suit.
        if out_dir != self.out_dir {
            tray::set_clips_dir(out_dir.clone());
            // La page ouverte se recharge et redemande la liste.
            ui::set_clips_dir(&out_dir);
            self.out_dir = out_dir;
        }
        if let Some(tray) = &self.tray {
            tray.set_tooltip(&tooltip(&next));
        }
        self.config = next;
        info!("réglage : {key} = {value}");
        self.config.save(&self.config_path).context(NOT_SAVED)
    }

    fn pause_hotkey(&mut self) -> Result<()> {
        if !self.hotkey_paused {
            // SAFETY: désenregistre le raccourci de notre fenêtre.
            unsafe { UnregisterHotKey(Some(self.window), HOTKEY_ID) }
                .context("UnregisterHotKey")?;
            self.hotkey_paused = true;
        }
        Ok(())
    }

    fn resume_hotkey(&mut self) -> Result<()> {
        if self.hotkey_paused {
            register_hotkey(self.window, &parse_hotkey(&self.config.hotkey)?)?;
            self.hotkey_paused = false;
        }
        Ok(())
    }

    /// Remplace le raccourci ; s'il est déjà pris, l'ancien est remis.
    fn replace_hotkey(&mut self, new: &str) -> Result<()> {
        let key = parse_hotkey(new)?;
        self.pause_hotkey()?;
        if let Err(e) = register_hotkey(self.window, &key) {
            self.resume_hotkey().context("ancien raccourci non remis")?;
            return Err(e.context(format!("raccourci {new} indisponible (déjà pris ?)")));
        }
        self.hotkey_paused = false;
        Ok(())
    }

    /// Nouveau format vidéo : buffer vidé (ses images n'ont plus le format de
    /// l'encodeur). Si le nouveau format échoue, l'ancien est relancé.
    fn restart_video(&mut self, next: &Config) -> Result<()> {
        // La sauvegarde en cours lit le buffer au format actuel.
        wait_for_save(&self.saving);
        self.video.stop();
        self.ring
            .lock()
            .map_err(|_| anyhow!("ring empoisonné"))?
            .clear();
        match spawn_video(next, &self.ring) {
            Ok(video) => {
                self.video = video;
                Ok(())
            }
            Err(e) => {
                match spawn_video(&self.config, &self.ring) {
                    Ok(video) => self.video = video,
                    Err(old) => {
                        // Plus de vidéo du tout : arrêt, comme pour une panne d'encodage.
                        error!("ancien format vidéo non relancé : {old:#}");
                        // SAFETY: termine la boucle de messages de ce thread (wParam 1).
                        unsafe { PostQuitMessage(1) };
                    }
                }
                Err(e)
            }
        }
    }

    /// Réglages et valeurs calculées, au format lu par `ui.html`.
    fn state(&self) -> String {
        let format = &self.video.format;
        let mut state = String::from("state\n");
        for (key, value) in self.config.pairs() {
            state.push_str(&format!("{key}={value}\n"));
        }
        let estimate = config::estimated_mb(format.bitrate, AUDIO_BPS, self.config.clip_seconds);
        state.push_str(&format!(
            "clips_dir={}\nstartup={}\nwidth={}\nestimate_mb={estimate:.0}\nbitrate_mbps={:.1}\nnew_since={}\n",
            self.out_dir.display(),
            tray::startup_enabled(),
            format.width,
            f64::from(format.bitrate) / 1e6,
            ui::new_since_ms(),
        ));
        // Micros branchés, en paires identifiant⇥nom.
        match audio::microphones() {
            Ok(mics) => {
                let list: Vec<String> = mics
                    .iter()
                    .map(|(id, name)| format!("{id}\t{name}"))
                    .collect();
                state.push_str(&format!("microphones={}\n", list.join("\t")));
            }
            Err(e) => warn!("liste des micros : {e:#}"),
        }
        state
    }
}

fn wait_for_save(saving: &AtomicBool) {
    let deadline = Instant::now() + SAVE_WAIT;
    while saving.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Fenêtre principale, invisible (message-only) : elle reçoit le raccourci et les
/// demandes de l'icône et de la fenêtre de réglages. Des messages de thread seraient
/// jetés par les boucles modales (menu de l'icône, sélecteur de dossier), qui
/// distribuent en revanche ceux des fenêtres.
fn create_main_window() -> Result<HWND> {
    // SAFETY: classe et fenêtre Win32 classiques, créées et utilisées sur ce thread.
    unsafe {
        let instance = GetModuleHandleW(None).context("GetModuleHandleW")?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(main_proc),
            hInstance: instance.into(),
            lpszClassName: w!("ClipperMain"),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            bail!("RegisterClassW (fenêtre principale)");
        }
        let window = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("ClipperMain"),
            None,
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            None,
        )
        .context("CreateWindowExW (fenêtre principale)")?;
        MAIN_WINDOW.store(window.0 as usize, Ordering::Relaxed);
        Ok(window)
    }
}

static MAIN_WINDOW: AtomicUsize = AtomicUsize::new(0);

/// Demande à la fenêtre principale (`tray::WM_MICROPHONE`, `ui::WM_UI`).
pub fn post_to_main(msg: u32) {
    let window = HWND(MAIN_WINDOW.load(Ordering::Relaxed) as _);
    // SAFETY: fenêtre principale vivante tant que la boucle de messages tourne.
    if let Err(e) = unsafe { PostMessageW(Some(window), msg, WPARAM(0), LPARAM(0)) } {
        error!("message {msg:#x} non transmis : {e:#}");
    }
}

extern "system" fn main_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_HOTKEY => with_app(|app| app.save()),
        tray::WM_MICROPHONE => with_app(|app| {
            let on = !app.config.microphone;
            app.change_and_report("microphone", &on.to_string());
        }),
        ui::WM_UI => {
            for request in ui::take_requests() {
                handle_ui(&request);
            }
        }
        WM_CLIPS_CHANGED if ui::current_hwnd().is_some() => handle_ui("clips"),
        // SAFETY: traitement par défaut des autres messages.
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

/// Durée gardée en mémoire : le clip + 1 GOP (il démarre à la keyframe qui précède).
fn keep(config: &Config) -> i64 {
    i64::from(config.clip_seconds) * SEC + SEC
}

fn spawn_video(config: &Config, ring: &Arc<Mutex<Ring>>) -> Result<Video> {
    // SAFETY: GetCurrentThreadId n'a pas de précondition.
    let main_thread = unsafe { GetCurrentThreadId() };
    let video = video::spawn(
        config.height,
        config.fps,
        config.quality,
        ring.clone(),
        main_thread,
    )?;
    info!(
        "taille estimée d'un clip : {:.0} Mo",
        config::estimated_mb(video.format.bitrate, AUDIO_BPS, config.clip_seconds)
    );
    Ok(video)
}

fn tooltip(config: &Config) -> String {
    format!(
        "Clipper — {} : {} dernières secondes",
        config.hotkey, config.clip_seconds
    )
}

/// Réglage appliqué mais pas écrit dans `clipper.toml`.
const NOT_SAVED: &str = "Réglage appliqué, mais non enregistré";

fn register_hotkey(window: HWND, hotkey: &Hotkey) -> Result<()> {
    // SAFETY: raccourci global lié à notre fenêtre principale ; pas de hook clavier.
    unsafe {
        RegisterHotKey(
            Some(window),
            HOTKEY_ID,
            HOT_KEY_MODIFIERS(hotkey.modifiers) | MOD_NOREPEAT,
            hotkey.vk,
        )
    }
    .context("RegisterHotKey")
}

fn save_clip(
    ring: &Mutex<Ring>,
    pressed: i64,
    duration: i64,
    video: &VideoFormat,
    audio: Option<&AudioFormat>,
    dir: &Path,
) -> Result<PathBuf> {
    mf::startup()?;
    // Les encodeurs ont du retard (quelques images ; l'audio jusqu'à ~100 ms quand une
    // source est muette) : on attend qu'ils aient sorti l'instant de l'appui (OBS fait
    // de même avec `save_ts`).
    let deadline = Instant::now() + CATCH_UP;
    let clip = loop {
        {
            let ring = ring.lock().map_err(|_| anyhow!("ring empoisonné"))?;
            let video_done = ring.last_ts().is_some_and(|ts| ts >= pressed);
            let audio_done =
                audio.is_none() || ring.last_audio_ts().is_some_and(|ts| ts >= pressed);
            if video_done && audio_done {
                break ring.snapshot(pressed, duration);
            }
        }
        if Instant::now() > deadline {
            bail!("les encodeurs n'ont pas rattrapé l'instant de l'appui");
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let v = &clip.video;
    let seconds = v.last().map_or(0, |l| l.ts - v[0].ts) as f64 / SEC as f64;
    info!(
        "{} images, {seconds:.2} s, {} trames audio, t0 = {} (QPC 100 ns)",
        v.len(),
        clip.audio.len(),
        v.first().map_or(0, |p| p.ts)
    );
    save::save(&clip, video, audio, dir)
}
