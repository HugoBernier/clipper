#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod config;
mod mf;
mod mix;
mod ring;
mod save;
mod tray;
mod ui;
mod video;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use log::{error, info, warn};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::System::Ole::OleInitialize;
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentThreadId};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MB_ICONHAND, MB_OK, MESSAGEBOX_STYLE, MSG, TranslateMessage,
    WM_HOTKEY,
};

use crate::audio::Microphone;
use crate::config::{Config, Hotkey, parse_hotkey};
use crate::mf::{AudioFormat, SEC, VideoFormat};
use crate::ring::Ring;
use crate::video::Video;

/// AAC 160 kb/s (débits possibles de l'encodeur Windows : 96, 128, 160, 192).
const AUDIO_BPS: u32 = 160_000;
/// Identifiant du raccourci global (un seul).
const HOTKEY_ID: i32 = 1;
/// Délai max d'une sauvegarde en cours quand on quitte.
const QUIT_WAIT: Duration = Duration::from_secs(10);
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
        beep(false);
    }));
    if let Err(e) = run() {
        error!("{e:#}");
        beep(false);
        return Err(e);
    }
    Ok(())
}

/// Log dans `%LOCALAPPDATA%\clipper\clipper.log`, recréé à chaque démarrage (pas de
/// console en release). En debug, aussi sur la sortie d'erreur.
/// `CLIPPER_DEBUG=1` : logs détaillés (diagnostic).
fn init_log() -> Result<()> {
    let dir = PathBuf::from(std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA absent")?)
        .join("clipper");
    std::fs::create_dir_all(&dir)?;
    // Nom distinct en debug : ne pas écraser le log de la version installée.
    let name = if cfg!(debug_assertions) {
        "clipper-debug.log"
    } else {
        "clipper.log"
    };
    let file = std::fs::File::create(dir.join(name))?;
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
    let microphone = Arc::new(Microphone::new(config.microphone, config.microphone_volume));
    // Sans audio (aucune sortie son, encodeur absent), on garde au moins la vidéo.
    let audio = audio::spawn(AUDIO_BPS, microphone.clone(), ring.clone())
        .inspect_err(|e| warn!("clips sans son : {e:#}"))
        .ok();
    register_hotkey(&hotkey)
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
    let mut app = App {
        config,
        config_path,
        out_dir,
        ring,
        video,
        microphone,
        tray,
    };

    let saving = Arc::new(AtomicBool::new(false));
    let mut msg = MSG::default();
    // SAFETY: boucle de messages standard du thread courant.
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        match msg.message {
            tray::WM_MICROPHONE => {
                let on = !app.config.microphone;
                app.change_and_report("microphone", &on.to_string());
            }
            ui::WM_UI => {
                for request in ui::take_requests() {
                    let reply = app.handle_ui(&request);
                    ui::post(&reply);
                }
            }
            WM_HOTKEY => app.save(&saving, audio.as_ref()),
            _ => {
                // Messages des fenêtres (icône, réglages).
                // SAFETY: message reçu par GetMessageW, transmis tel quel.
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
    }
    // WM_QUIT avec wParam 1 : le thread vidéo s'est arrêté sur une erreur (déjà loguée).
    if msg.wParam.0 == 1 {
        bail!("arrêt : erreur vidéo");
    }
    // « Quitter » : on laisse finir une sauvegarde en cours.
    let deadline = Instant::now() + QUIT_WAIT;
    while saving.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
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
}

impl App {
    /// Appui sur le raccourci : sauvegarde dans un thread, une seule à la fois.
    fn save(&self, saving: &Arc<AtomicBool>, audio: Option<&AudioFormat>) {
        let pressed = mf::now();
        // Comme OBS : une seule sauvegarde à la fois, l'appui suivant est ignoré.
        if saving.swap(true, Ordering::SeqCst) {
            warn!("sauvegarde déjà en cours, appui ignoré");
            return;
        }
        let clip = i64::from(self.config.clip_seconds) * SEC;
        let (ring, saving, out_dir, audio, format) = (
            self.ring.clone(),
            saving.clone(),
            self.out_dir.clone(),
            audio.cloned(),
            self.video.format,
        );
        std::thread::spawn(move || {
            match save_clip(&ring, pressed, clip, &format, audio.as_ref(), &out_dir) {
                Ok(path) => {
                    info!("clip sauvegardé : {}", path.display());
                    beep(true);
                }
                Err(e) => {
                    error!("échec de la sauvegarde : {e:#}");
                    beep(false);
                }
            }
            saving.store(false, Ordering::SeqCst);
        });
    }

    /// Message de la fenêtre ; la réponse est toujours l'état complet, avec l'erreur
    /// éventuelle.
    fn handle_ui(&mut self, request: &str) -> String {
        let (command, arg) = request.split_once('\n').unwrap_or((request, ""));
        let result = match command {
            "get" => Ok(()),
            "set" => match arg.split_once('=') {
                Some((key, value)) => self.change(key, value),
                None => Err(anyhow!("réglage mal formé : {arg}")),
            },
            "pick_folder" => ui::pick_folder(&self.out_dir).and_then(|picked| match picked {
                Some(dir) => self.change("output_dir", &dir.display().to_string()),
                None => Ok(()),
            }),
            "startup" => tray::set_startup(arg == "true"),
            _ => Err(anyhow!("commande inconnue : {command}")),
        };
        let mut state = self.state();
        if let Err(e) = result {
            warn!("réglage refusé : {e:#}");
            state.push_str(&format!("error={e:#}\n"));
        }
        state
    }

    /// Changement venu d'ailleurs que la fenêtre (menu de l'icône) : la fenêtre, si elle
    /// est ouverte, suit.
    fn change_and_report(&mut self, key: &str, value: &str) {
        if let Err(e) = self.change(key, value) {
            error!("réglage {key} : {e:#}");
            beep(false);
        }
        ui::post(&self.state());
    }

    /// Applique un réglage sans relancer Clipper, puis l'enregistre. Seul ce qui dépend
    /// du réglage redémarre : la vidéo pour la résolution, les fps ou la qualité ;
    /// l'audio n'est jamais interrompu.
    fn change(&mut self, key: &str, value: &str) -> Result<()> {
        let mut next = self.config.clone();
        next.set(key, value)?;
        if next.hotkey != self.config.hotkey {
            change_hotkey(&self.config.hotkey, &next.hotkey)?;
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
        tray::set_microphone(next.microphone);
        self.out_dir = save::videos_dir()?.join(&next.output_dir);
        tray::set_clips_dir(self.out_dir.clone());
        if let Some(tray) = &self.tray {
            tray.set_tooltip(&tooltip(&next));
        }
        self.config = next;
        info!("réglage : {key} = {value}");
        self.config.save(&self.config_path)
    }

    /// Nouveau format vidéo : buffer vidé (ses images n'ont plus le format de
    /// l'encodeur). Si le nouveau format échoue, l'ancien est relancé.
    fn restart_video(&mut self, next: &Config) -> Result<()> {
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
                self.video = spawn_video(&self.config, &self.ring)
                    .context("ancien format vidéo non relancé")?;
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
            "clips_dir={}\nstartup={}\nwidth={}\nestimate_mb={estimate:.0}\nbitrate_mbps={:.1}\n",
            self.out_dir.display(),
            tray::startup_enabled(),
            format.width,
            f64::from(format.bitrate) / 1e6,
        ));
        state
    }
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

fn register_hotkey(hotkey: &Hotkey) -> Result<()> {
    // SAFETY: raccourci global lié au thread courant (hwnd nul) ; pas de hook clavier.
    unsafe {
        RegisterHotKey(
            None,
            HOTKEY_ID,
            HOT_KEY_MODIFIERS(hotkey.modifiers) | MOD_NOREPEAT,
            hotkey.vk,
        )
    }
    .context("RegisterHotKey")
}

/// Remplace le raccourci ; s'il est déjà pris, l'ancien est remis.
fn change_hotkey(old: &str, new: &str) -> Result<()> {
    let new_key = parse_hotkey(new)?;
    // SAFETY: désenregistre le raccourci de ce thread.
    unsafe { UnregisterHotKey(None, HOTKEY_ID) }.context("UnregisterHotKey")?;
    if let Err(e) = register_hotkey(&new_key) {
        register_hotkey(&parse_hotkey(old)?).context("ancien raccourci non remis")?;
        return Err(e.context(format!("raccourci {new} indisponible (déjà pris ?)")));
    }
    Ok(())
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
