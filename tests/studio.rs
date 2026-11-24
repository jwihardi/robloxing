use anyhow::Result;
use tempfile::tempdir;
use tokio::sync::oneshot;
use workshop::{Store, studio::Recipe};

#[test]
fn library_persists_ready_assets_and_recovers_interrupted_runs() -> Result<()> {
    let tmp = tempdir()?;
    let project = tmp.path().join("game");
    let home = tmp.path().join("data");
    let tile = Recipe::Texture {
        material: "Moss".into(),
        seed: 48,
    };
    let store = Store::open(&project, &home)?;
    let id = store.start_studio_job(&tile)?;
    let (_keep, cancel) = oneshot::channel();
    store.run_studio_job(id, cancel)?;
    let job = store.studio_job(id)?;
    assert_eq!(job.state, "Ready");
    let asset = job.folder(&store).join(job.filename());
    let bytes = std::fs::read(&asset)?;
    assert_eq!(image::image_dimensions(&asset)?, (256, 256));
    assert!(!project.join("image.png").exists());

    let cancelled = store.start_studio_job(&tile)?;
    let (stop, signal) = oneshot::channel();
    stop.send(()).unwrap();
    assert!(store.run_studio_job(cancelled, signal).is_err());
    assert_eq!(store.studio_job(cancelled)?.state, "Cancelled");
    let interrupted = store.start_studio_job(&tile)?;
    drop(store);

    let store = Store::open(&project, &home)?;
    assert_eq!(store.studio_job(interrupted)?.state, "Interrupted");
    assert_eq!(std::fs::read(asset)?, bytes);
    assert_eq!(store.studio_history("Images")?.len(), 3);
    let art = store.find_studio_jobs("Images", "mOsS", true, Some(12))?;
    assert_eq!(art.len(), 1);
    assert_eq!(art[0].id, id);
    assert!(
        store
            .find_studio_jobs("Images", "Basalt", false, None)?
            .is_empty()
    );
    assert!(store.studio_history("Models")?.is_empty());
    let other = Store::open(&tmp.path().join("other"), &home)?;
    assert!(other.studio_history("Images")?.is_empty());
    Ok(())
}
