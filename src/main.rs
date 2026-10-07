mod config;
mod mf;
mod ring;
mod save;
mod video;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use log::{error, info, warn};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_NOREPEAT, RegisterHotKey,
};
use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};

use crate::config::{Config, parse_hotkey};
use crate::mf::{SEC, VideoFormat};
use crate::ring::Ring;

/// Débit réservé dès maintenant à l'audio (itération 2), pour que la taille ne bouge pas.
const AUDIO_BPS: u32 = 160_000;
/// Délai max pour que l'encodeur rattrape l'instant de l'appui.
const CATCH_UP: Duration = Duration::from_secs(2);

fn main() -> Result<()> {
    simplelog::SimpleLogger::init(log::LevelFilter::Info, simplelog::Config::default())?;
    if let Err(e) = run() {
        error!("{e:#}");
        return Err(e);
    }
    Ok(())
}

fn run() -> Result<()> {
    let exe = std::env::current_exe()?;
    let exe_dir = exe.parent().context("dossier de l'exe")?;
    let config = Config::load_or_create(&exe_dir.join("clipper.toml"))?;
    let hotkey = parse_hotkey(&config.hotkey)?;
    let bitrate = config.video_bitrate(AUDIO_BPS)?;
    let clip = i64::from(config.clip_seconds) * SEC;
    let out_dir = exe_dir.join(&config.output_dir);

    // + 1 GOP : le clip démarre à la keyframe qui précède le début voulu.
    let ring = Arc::new(Mutex::new(Ring::new(clip + SEC)));
    // SAFETY: GetCurrentThreadId n'a pas de précondition.
    let main_thread = unsafe { GetCurrentThreadId() };
    let format = video::spawn(
        config.height,
        config.fps,
        bitrate,
        ring.clone(),
        main_thread,
    )?;

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

    let saving = Arc::new(AtomicBool::new(false));
    let mut msg = MSG::default();
    // SAFETY: boucle de messages standard du thread courant.
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        if msg.message != WM_HOTKEY {
            continue;
        }
        let pressed = mf::now();
        // Comme OBS : une seule sauvegarde à la fois, l'appui suivant est ignoré.
        if saving.swap(true, Ordering::SeqCst) {
            warn!("sauvegarde déjà en cours, appui ignoré");
            continue;
        }
        let (ring, saving, out_dir) = (ring.clone(), saving.clone(), out_dir.clone());
        std::thread::spawn(move || {
            match save_clip(&ring, pressed, clip, &format, &out_dir) {
                Ok(path) => info!("clip sauvegardé : {}", path.display()),
                Err(e) => error!("échec de la sauvegarde : {e:#}"),
            }
            saving.store(false, Ordering::SeqCst);
        });
    }
    // WM_QUIT avec wParam 1 : le thread vidéo s'est arrêté sur une erreur (déjà loguée).
    if msg.wParam.0 == 1 {
        bail!("arrêt : erreur vidéo");
    }
    Ok(())
}

fn save_clip(
    ring: &Mutex<Ring>,
    pressed: i64,
    clip: i64,
    format: &VideoFormat,
    dir: &Path,
) -> Result<PathBuf> {
    mf::startup()?;
    // L'encodeur a quelques images de retard : on attend qu'il ait sorti l'instant de
    // l'appui (OBS fait de même avec `save_ts`).
    let deadline = Instant::now() + CATCH_UP;
    let packets = loop {
        {
            let ring = ring.lock().map_err(|_| anyhow!("ring empoisonné"))?;
            if ring.last_ts().is_some_and(|ts| ts >= pressed) {
                break ring.snapshot(pressed, clip);
            }
        }
        if Instant::now() > deadline {
            bail!("l'encodeur n'a pas rattrapé l'instant de l'appui");
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let seconds = packets.last().map_or(0, |l| l.ts - packets[0].ts) as f64 / SEC as f64;
    info!("{} images, {seconds:.2} s", packets.len());
    save::save(&packets, format, dir)
}
