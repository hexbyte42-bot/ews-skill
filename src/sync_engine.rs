use crate::cache::{CachedEmail, CachedFolder, Repository, SyncState};
use crate::config::Config;
use crate::ews_client::{distinguished_folder_id_from_spec, EwsClient};
use chrono::Utc;
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

#[derive(Default)]
struct PollControl {
    running: bool,
    stop_tx: Option<mpsc::Sender<()>>,
}

pub struct SyncEngine {
    ews_client: EwsClient,
    repository: Repository,
    config: Config,
    poll_control: Arc<Mutex<PollControl>>,
}

impl SyncEngine {
    pub fn new(ews_client: EwsClient, repository: Repository, config: Config) -> Self {
        Self {
            ews_client,
            repository,
            config,
            poll_control: Arc::new(Mutex::new(PollControl::default())),
        }
    }

    pub fn get_client(&self) -> &EwsClient {
        &self.ews_client
    }

    pub async fn initialize(&self) -> Result<(), String> {
        info!("Initializing sync engine");

        for folder_name in &self.config.sync.folders {
            match self.find_and_cache_folder(folder_name).await {
                Ok(Some(folder)) => info!(
                    "Found and cached folder: {} ({})",
                    folder.display_name, folder.id
                ),
                Ok(None) => warn!("Folder not found: {}", folder_name),
                Err(e) => error!("Error finding folder {}: {}", folder_name, e),
            }
        }

        if self.config.sync.initial_sync {
            self.full_sync_all_folders().await?;
        }

        Ok(())
    }

    pub async fn find_and_cache_folder(&self, name: &str) -> Result<Option<CachedFolder>, String> {
        let folder = self
            .ews_client
            .find_folder(name)
            .await
            .map_err(|e| e.to_string())?;

        if let Some(f) = folder {
            let canonical = distinguished_folder_id_from_spec(name).unwrap_or(name);
            let cached = CachedFolder {
                id: f.folder_id.id,
                change_key: Some(f.folder_id.change_key),
                parent_id: None,
                display_name: canonical.to_string(),
                unread_count: f.unread_count,
                total_count: f.total_count,
                synced_at: Utc::now(),
            };
            self.repository.save_folder(&cached);
            Ok(Some(cached))
        } else {
            Ok(None)
        }
    }

    pub async fn full_sync_all_folders(&self) -> Result<(), String> {
        for folder in self.repository.list_folders() {
            if let Err(e) = self.full_sync_folder(&folder.id).await {
                error!("Error syncing folder {}: {}", folder.id, e);
            }
        }
        Ok(())
    }

    pub async fn full_sync_folder(&self, folder_id: &str) -> Result<(), String> {
        info!("Starting full sync for folder: {}", folder_id);
        let response = self
            .ews_client
            .sync_folder_items(folder_id, None, 512)
            .await
            .map_err(|e| e.to_string())?;
        self.apply_sync_response(folder_id, response).await;
        info!("Full sync completed for folder: {}", folder_id);
        Ok(())
    }

    pub async fn incremental_sync(&self, folder_id: &str) -> Result<(), String> {
        let state = self
            .repository
            .get_sync_state(folder_id)
            .map(|s| s.sync_state);
        let response = self
            .ews_client
            .sync_folder_items(folder_id, state, 512)
            .await
            .map_err(|e| e.to_string())?;
        self.apply_sync_response(folder_id, response).await;
        Ok(())
    }

    pub async fn sync_all_folders(&self) -> Result<(), String> {
        for folder in self.repository.list_folders() {
            if let Err(e) = self.incremental_sync(&folder.id).await {
                error!("Error in incremental sync for folder {}: {}", folder.id, e);
            }
        }
        Ok(())
    }

    pub fn start_polling(&self, runtime: &Runtime) {
        let mut control = self.poll_control.lock();
        if control.running {
            return;
        }

        let (tx, mut rx) = mpsc::channel::<()>(1);
        control.running = true;
        control.stop_tx = Some(tx);
        drop(control);

        let ews_client = self.ews_client.clone();
        let repo = self.repository.clone();
        let interval = self.config.sync.interval_seconds;
        let poll_control = self.poll_control.clone();

        runtime.spawn(async move {
            loop {
                tokio::select! {
                    _ = rx.recv() => break,
                    _ = tokio::time::sleep(tokio::time::Duration::from_secs(interval)) => {
                        for folder in repo.list_folders() {
                            let folder_id = folder.id;
                            let state = repo.get_sync_state(&folder_id).map(|s| s.sync_state);

                            match ews_client.sync_folder_items(&folder_id, state, 512).await {
                                Ok(response) => {
                                    apply_sync_response_to_repo(&repo, &ews_client, &folder_id, response).await;
                                }
                                Err(e) => {
                                    error!("Sync error for folder {}: {}", folder_id, e);
                                }
                            }
                        }
                    }
                }

                if !poll_control.lock().running {
                    break;
                }
            }
        });
    }

    pub fn stop_polling(&self) {
        let tx = {
            let mut control = self.poll_control.lock();
            control.running = false;
            control.stop_tx.take()
        };

        if let Some(tx) = tx {
            let _ = tx.try_send(());
        }
    }

    async fn apply_sync_response(
        &self,
        folder_id: &str,
        response: crate::ews_client::SyncFolderItemsResponse,
    ) {
        apply_sync_response_to_repo(&self.repository, &self.ews_client, folder_id, response).await;
    }
}

async fn apply_sync_response_to_repo(
    repository: &Repository,
    ews_client: &EwsClient,
    folder_id: &str,
    response: crate::ews_client::SyncFolderItemsResponse,
) {
    let sync_response = response.response_messages.sync_folder_items;

    if let Some(sync_state) = sync_response.sync_state {
        repository.save_sync_state(&SyncState {
            folder_id: folder_id.to_string(),
            sync_state,
            last_sync_at: Utc::now(),
        });
    }

    if let Some(creates) = sync_response.changes.create {
        for change in creates {
            for mut email in change.messages {
                if needs_enrichment(&email) && !email.item_id.id.trim().is_empty() {
                    if let Ok(full) = ews_client.get_item(&email.item_id.id).await {
                        email = full;
                    }
                }
                repository.save_email(&CachedEmail::from_ews_email(&email, folder_id));
            }
        }
    }

    if let Some(updates) = sync_response.changes.update {
        for change in updates {
            for mut email in change.messages {
                if needs_enrichment(&email) && !email.item_id.id.trim().is_empty() {
                    if let Ok(full) = ews_client.get_item(&email.item_id.id).await {
                        email = full;
                    }
                }
                repository.save_email(&CachedEmail::from_ews_email(&email, folder_id));
            }
        }
    }

    if let Some(deletes) = sync_response.changes.delete {
        for delete in deletes {
            repository.delete_email(&delete.item_id.id);
        }
    }
}

fn needs_enrichment(email: &crate::ews_client::Email) -> bool {
    email.sender.mailbox.is_none()
        || email.datetime_received.trim().is_empty()
        || email.importance.trim().is_empty()
        || (email.body.value.trim().is_empty() && email.text_body.value.trim().is_empty())
}

impl Clone for SyncEngine {
    fn clone(&self) -> Self {
        Self {
            ews_client: self.ews_client.clone(),
            repository: self.repository.clone(),
            config: self.config.clone(),
            poll_control: self.poll_control.clone(),
        }
    }
}
