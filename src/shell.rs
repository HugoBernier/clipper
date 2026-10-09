//! Actions de Windows sur un clip, comme dans l'Explorateur : copier, glisser, mettre à
//! la corbeille, montrer dans son dossier ; et ouvrir le dossier des clips. À appeler
//! sur le thread principal (STA).

use std::path::Path;

use anyhow::{Context, Result, bail};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IDataObject};
use windows::Win32::System::Ole::{
    DROPEFFECT_COPY, IDropSource, OleFlushClipboard, OleSetClipboard,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_LBUTTON};
use windows::Win32::UI::Shell::{
    BHID_DataObject, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT,
    FOF_WANTNUKEWARNING, FOFX_RECYCLEONDELETE, FileOperation, IFileOperation,
    IFileOperationProgressSink, ILCreateFromPathW, ILFree, IShellItem, SHCreateItemFromParsingName,
    SHDoDragDrop, SHOpenFolderAndSelectItems,
};
use windows::core::HSTRING;

/// Le fichier dans le presse-papiers, comme un Ctrl+C dans l'Explorateur : un Ctrl+V
/// dans Discord l'envoie.
pub fn copy(path: &Path) -> Result<()> {
    let data = data_object(path)?;
    // SAFETY: objet de données du Shell ; le flush le rend au presse-papiers, qui ne
    // dépend plus de nous (le clip reste collable si Clipper s'arrête).
    unsafe {
        OleSetClipboard(&data).context("OleSetClipboard")?;
        OleFlushClipboard().context("OleFlushClipboard")?;
    }
    Ok(())
}

/// Glisser du fichier vers Discord ou l'Explorateur (boucle modale jusqu'au dépôt).
pub fn drag(owner: Option<HWND>, path: &Path) -> Result<()> {
    // La page demande le glisser de façon asynchrone : le bouton a pu être relâché
    // entre-temps, et le Shell déposerait aussitôt sous le curseur.
    // SAFETY: GetKeyState n'a pas de précondition.
    if unsafe { GetKeyState(i32::from(VK_LBUTTON.0)) } >= 0 {
        return Ok(());
    }
    let data = data_object(path)?;
    // SAFETY: sans IDropSource, le Shell fournit le sien (curseurs, Échap pour annuler).
    unsafe { SHDoDragDrop(owner, &data, None::<&IDropSource>, DROPEFFECT_COPY) }
        .context("SHDoDragDrop")?;
    Ok(())
}

/// Vers la corbeille, donc récupérable. Sans corbeille (clé USB, partage réseau),
/// Windows demande confirmation avant de supprimer pour de bon.
pub fn recycle(owner: Option<HWND>, path: &Path) -> Result<()> {
    // SAFETY: objets COM du Shell, créés et utilisés sur ce thread (STA).
    unsafe {
        let operation: IFileOperation =
            CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)
                .context("FileOperation")?;
        operation
            .SetOperationFlags(
                FOF_ALLOWUNDO
                    | FOF_NOCONFIRMATION
                    | FOF_WANTNUKEWARNING
                    | FOF_SILENT
                    | FOF_NOERRORUI
                    | FOFX_RECYCLEONDELETE,
            )
            .context("SetOperationFlags")?;
        if let Some(owner) = owner {
            operation.SetOwnerWindow(owner).context("SetOwnerWindow")?;
        }
        operation
            .DeleteItem(&shell_item(path)?, None::<&IFileOperationProgressSink>)
            .context("DeleteItem")?;
        operation
            .PerformOperations()
            .context("mise à la corbeille (clip en cours d'utilisation ?)")?;
        if operation
            .GetAnyOperationsAborted()
            .context("GetAnyOperationsAborted")?
            .as_bool()
        {
            bail!("mise à la corbeille annulée");
        }
    }
    Ok(())
}

/// Ouvre le dossier dans l'Explorateur, le clip sélectionné.
pub fn reveal(path: &Path) -> Result<()> {
    // SAFETY: liste d'identifiants créée puis libérée ici ; un seul élément désigné
    // entièrement (cidl = 0), comme le prévoit SHOpenFolderAndSelectItems.
    unsafe {
        let pidl = ILCreateFromPathW(&HSTRING::from(path.as_os_str()));
        if pidl.is_null() {
            bail!("clip introuvable : {}", path.display());
        }
        let result = SHOpenFolderAndSelectItems(pidl, None, 0);
        ILFree(Some(pidl));
        result.context("SHOpenFolderAndSelectItems")
    }
}

/// Ouvre le dossier des clips dans l'Explorateur (créé s'il n'existe pas encore).
pub fn open_folder(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("création de {}", dir.display()))?;
    std::process::Command::new("explorer")
        .arg(dir)
        .spawn()
        .context("lancement de l'Explorateur")?;
    Ok(())
}

fn shell_item(path: &Path) -> Result<IShellItem> {
    // SAFETY: chemin valide pendant l'appel.
    unsafe { SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None) }
        .with_context(|| format!("clip introuvable : {}", path.display()))
}

/// Le même objet que l'Explorateur fournit pour un Ctrl+C ou un glisser.
fn data_object(path: &Path) -> Result<IDataObject> {
    // SAFETY: élément du Shell valide.
    unsafe { shell_item(path)?.BindToHandler(None, &BHID_DataObject) }
        .context("BindToHandler(DataObject)")
}
