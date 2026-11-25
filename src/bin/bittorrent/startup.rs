use anyhow::{Result, bail};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::Path;
use std::sync::Arc;
use tokio::signal;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;

use bittorrent_rs::dht::Dht;
use bittorrent_rs::session::{Client, Session};
use bittorrent_rs::storage::{ResumeData, Storage};
use bittorrent_rs::torrent::{Magnet, Torrent};
use bittorrent_rs::tracker::{Event, announce_all};
use bittorrent_rs::transfer::metadata::resolve;
use bittorrent_rs::transfer::{download, seed};

use crate::args::Args;

const PEER_QUEUE: usize = 64;

pub async fn start() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .without_time()
        .with_target(false)
        .init();

    let args = Args::get();

    let client = Client::build()?;
    let port = client.port()?;
    let dht = Dht::start()
        .inspect_err(|e| warn!("DHT is unavailable: {e:#}"))
        .ok();
    let torrent = load(&args.path, &client, port, dht.as_ref()).await?;
    let resume_path = args
        .output
        .join(format!("{}.fastresume", torrent.meta.info.name));
    let storage = Storage::create(&args.output, &torrent.meta.info).await?;
    let session = Arc::new(Session::new(torrent, client.id, storage));

    let saved = ResumeData::load(&resume_path)
        .await
        .inspect_err(|e| debug!("No fastresume data: {e:#}"))
        .ok();
    let resumed = session.resume(saved).await?;
    if resumed > 0 {
        info!(
            "Resumed {resumed}/{} pieces from disk",
            session.torrent.meta.info.piece_count()
        );
    }

    let result = run(&session, &client, port, dht.as_ref()).await;

    if let Err(e) = save(&session, &resume_path).await {
        warn!("Failed to save fastresume data: {e:#}");
    }
    announce(&session, port, Event::Stopped).await;

    result
}

async fn load(source: &str, client: &Client, port: u16, dht: Option<&Dht>) -> Result<Torrent> {
    if !source.starts_with("magnet:") {
        return Torrent::load(source);
    }

    let magnet: Magnet = source.parse()?;
    tokio::select! {
        torrent = resolve(&magnet, client.id, port, dht) => torrent,
        result = signal::ctrl_c() => {
            result?;
            bail!("Interrupted while fetching metadata")
        }
    }
}

async fn run(session: &Arc<Session>, client: &Client, port: u16, dht: Option<&Dht>) -> Result<()> {
    let already_complete = session.is_complete();
    let progress = ProgressBar::new(session.torrent.meta.info.length)
        .with_position(session.verified_bytes())
        .with_style(ProgressStyle::with_template(
            "{bar:40} {bytes}/{total_bytes} {bytes_per_sec} eta {eta}",
        )?);

    let (peers_sender, peers) = mpsc::channel(PEER_QUEUE);
    let announcing = session.run_announcer(port, peers_sender.clone(), dht);
    let downloading = download(Arc::clone(session), peers, |len| progress.inc(len));
    let seeding = seed(Arc::clone(session), &client.lst, peers_sender);
    tokio::pin!(announcing, seeding);

    tokio::select! {
        never = &mut announcing => match never {},
        result = &mut seeding => result?,
        result = downloading => {
            result?;
            progress.finish();
            if !already_complete {
                announce(session, port, Event::Completed).await;
            }
            info!("Download complete, seeding until Ctrl+C");

            tokio::select! {
                never = announcing => match never {},
                result = seeding => result?,
                result = signal::ctrl_c() => result?,
            }
        }
        result = signal::ctrl_c() => result?,
    }

    Ok(())
}

async fn save(session: &Session, path: &Path) -> Result<()> {
    session.resume_data().await?.save(path).await
}

async fn announce(session: &Session, port: u16, event: Event) {
    announce_all(
        &session.torrent.meta.trackers,
        session.announce(port, Some(event)),
    )
    .await;
}
