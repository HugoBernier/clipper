#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod config;
mod mf;
mod mix;
mod ring;
mod save;
mod tray;
mod video;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use log::{error, info, warn};
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_NOREPEAT, RegisterHotKey,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MB_ICONHAND, MB_OK, MESSAGEBOX_STYLE, MSG, TranslateMessage,
    WM_HOTKEY,
};

use crate::config::{Config, parse_hotkey};
use crate::mf::{AudioFormat, SEC, VideoFormat};
use crate::ring::Ring;

/// AAC 160 kb/s (débits possibles de l'encodeur Windows : 96, 128, 160, 192).
const AUDIO_BPS: u32 = 160_000;
/// Délai max d'une sauvegarde en cours quand on quitte.
const QUIT_WAIT: Duration = Duration::from_secs(10);
/// Délai max pour que l'encodeur rattrape l'instant de l'appui.
const CATCH_UP: Duration = Duration::from_secs(2);

fn main() -> Result<()> {
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

/// Seul retour sans interface : un son de succès, un son d'erreur.
fn beep(ok: bool) {
    let sound: MESSAGEBOX_STYLE = if ok { MB_OK } else { MB_ICONHAND };
    // SAFETY: MessageBeep n'a pas de précondition.
    let _ = unsafe { MessageBeep(sound) };
}

fn run() -> Result<()> {
    let exe = std::env::current_exe()?;
    let exe_dir = exe.parent().context("dossier de l'exe")?;
    let config = Config::load_or_create(&exe_dir.join("clipper.toml"))?;
    let hotkey = parse_hotkey(&config.hotkey)?;
    let bitrate = config.video_bitrate(AUDIO_BPS)?;
    let clip = i64::from(config.clip_seconds) * SEC;
    let out_dir = save::videos_dir()?.join(&config.output_dir);

    // + 1 GOP : le clip démarre à la keyframe qui précède le début voulu.
    let ring = Arc::new(Mutex::new(Ring::new(clip + SEC)));
    // SAFETY: GetCurrentThreadId n'a pas de précondition.
    let main_thread = unsafe { GetCurrentThreadId() };
    let video = video::spawn(
        config.height,
        config.fps,
        bitrate,
        ring.clone(),
        main_thread,
    )?;
    // Sans audio (aucune sortie son, encodeur absent), on garde au moins la vidéo.
    let audio = audio::spawn(AUDIO_BPS, config.microphone, ring.clone())
        .inspect_err(|e| warn!("clips sans son : {e:#}"))
        .ok();

    // SAFETY: raccourci global lié au thread courant (hwnd nul) ; pas de hook clavier.
    unsafe {
        RegisterHotKey(
            None,
            1,
            HOT_KEY_MODIFIERS(hotkey.modifiers) | MOD_NOREPEAT,
            hotkey.vk,
        )
    }
    .with_context(|| format!("raccourci {} indisponible (déjà pris ?)", config.hotkey))?;
    info!(
        "prêt : {} sauvegarde les {} dernières secondes dans {}",
        config.hotkey,
        config.clip_seconds,
        out_dir.display()
    );
    // Sans icône (Explorateur absent…), Clipper reste utilisable au raccourci.
    let _tray = tray::Tray::new(
        out_dir.clone(),
        format!(
            "Clipper — {} : {} dernières secondes",
            config.hotkey, config.clip_seconds
        ),
    )
    .inspect_err(|e| warn!("icône de notification indisponible : {e:#}"))
    .ok();

    let saving = Arc::new(AtomicBool::new(false));
    let mut msg = MSG::default();
    // SAFETY: boucle de messages standard du thread courant.
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        if msg.message != WM_HOTKEY {
            // Messages de la fenêtre cachée de l'icône.
            // SAFETY: message reçu par GetMessageW, transmis tel quel.
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            continue;
        }
        let pressed = mf::now();
        // Comme OBS : une seule sauvegarde à la fois, l'appui suivant est ignoré.
        if saving.swap(true, Ordering::SeqCst) {
            warn!("sauvegarde déjà en cours, appui ignoré");
            continue;
        }
        let (ring, saving, out_dir, audio) =
            (ring.clone(), saving.clone(), out_dir.clone(), audio.clone());
        std::thread::spawn(move || {
            match save_clip(&ring, pressed, clip, &video, audio.as_ref(), &out_dir) {
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
