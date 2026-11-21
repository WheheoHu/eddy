//! An engine in this process, on a data directory of its own: what the
//! tests, development and Windows use, and where a data directory is
//! paired with a server or erased.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eddy_core::{Engine, EngineOptions, GalleryScope, Platform, SignOut};

use crate::cli::{DevCommand, Local};

/// A keychain for a data directory that is not the app's: secrets in
/// `cli-secrets.json` inside it. The file is mode 0600 on Unix; on Windows
/// it inherits the directory's ACL.
struct FileSecrets(PathBuf);

impl FileSecrets {
    fn load(&self) -> std::collections::HashMap<String, String> {
        std::fs::read(&self.0)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self, map: &std::collections::HashMap<String, String>) -> bool {
        let Ok(bytes) = serde_json::to_vec(map) else {
            return false;
        };
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(&self.0)
            .and_then(|mut f| std::io::Write::write_all(&mut f, &bytes))
            .is_ok()
    }
}

impl Platform for FileSecrets {
    fn secret_get(&self, key: &str) -> Option<String> {
        self.load().get(key).cloned()
    }

    fn secret_set(&self, key: &str, value: &str) -> bool {
        let mut map = self.load();
        map.insert(key.into(), value.into());
        self.save(&map)
    }

    fn secret_delete(&self, key: &str) {
        let mut map = self.load();
        map.remove(key);
        self.save(&map);
    }

    fn secret_clear(&self) {
        self.save(&Default::default());
    }
}

/// Opens (creating it if need be) the data directory `data`.
pub fn open(data: &Path) -> Result<Engine, String> {
    std::fs::create_dir_all(data)
        .map_err(|e| format!("cannot use {} for data: {e}", data.display()))?;
    let secrets = Arc::new(FileSecrets(data.join("cli-secrets.json")));
    let engine = Engine::open(data, secrets, EngineOptions::default())
        .map_err(|e| format!("cannot open {}: {e}", data.display()))?;
    engine.set_pdf_reader(Some(Arc::new(eddy_pdf::PdfiumReader::new())));
    Ok(engine)
}

/// Carries out a command that only exists here, printing as it goes.
pub async fn run(engine: &Engine, command: Local) -> eddy_core::Result<()> {
    match command {
        Local::Connect { url, code } => {
            let server = engine.connect_server(&url, &code, "eddy-cli").await?;
            println!(
                "connected to {} (server {}), cursor {}",
                server.url, server.server_id, server.cursor
            );
        }
        Local::Disconnect => engine.disconnect_server().await?,
        Local::SignOut { discard_unsent } => {
            match engine.sign_out_and_erase(discard_unsent).await? {
                SignOut::Erased => println!("signed out and erased"),
                SignOut::Unsent(n) => println!(
                    "not signed out: {n} changes could not be sent (sign-out --discard-unsent loses them)"
                ),
            }
        }
        // Standalone: this directory's FreshRSS account. Paired with a
        // server: the same account, kept here for refreshing while the
        // server is offline.
        Local::Freshrss {
            url,
            user,
            password,
        } => {
            let account = engine.configure_freshrss(&url, &user, &password).await?;
            println!("FreshRSS {} @ {}", account.username, account.base_url);
        }
        Local::ServerAccount {
            url,
            user,
            password,
        } => {
            engine
                .configure_server_freshrss(&url, &user, &password)
                .await?;
            println!("server account set");
        }
        // For checking hosts that guard their pictures.
        Local::Dev(DevCommand::Image { url, page }) => {
            let (bytes, mime) = engine.image(&url, page.as_deref()).await?;
            println!("{mime}  {} bytes", bytes.len());
        }
        Local::Dev(DevCommand::Media { id }) => {
            let set = engine.resolve_media(&id, true).await?;
            println!("{:?}  thumbnail {:?}", set.status, set.thumbnail);
            for m in set.items {
                println!(
                    "  {:?} {}  poster {:?}  {:?}x{:?}  {:?}s",
                    m.kind, m.src, m.poster, m.width, m.height, m.duration
                );
            }
        }
        Local::Dev(DevCommand::GalleryProbe { folder, count }) => {
            engine.set_folder_gallery(&folder, true)?;
            let entries =
                engine.gallery_items(&GalleryScope::Folder(folder.clone()), false, count, 0)?;
            let mut tally = std::collections::BTreeMap::new();
            for entry in entries {
                let host = entry
                    .row
                    .item
                    .url
                    .as_deref()
                    .and_then(|u| url::Url::parse(u).ok())
                    .and_then(|u| u.host_str().map(String::from))
                    .unwrap_or_default();
                let key = match engine.resolve_media(&entry.row.item.id, true).await {
                    Ok(set) => format!(
                        "{host:<22} {:?} {:?}",
                        set.status,
                        set.items.first().map(|m| m.kind)
                    ),
                    Err(e) => format!("{host:<22} error {e}"),
                };
                *tally.entry(key).or_insert(0) += 1;
            }
            for (key, n) in tally {
                println!("{n:>4}  {key}");
            }
        }
    }
    Ok(())
}
