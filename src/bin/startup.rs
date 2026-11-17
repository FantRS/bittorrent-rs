use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::signal;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use bittorrent_rs::args_parser::Args;
use bittorrent_rs::client::{Client, Event};
use bittorrent_rs::download::download;
use bittorrent_rs::session::Session;
use bittorrent_rs::storage::Storage;
use bittorrent_rs::torrent::Torrent;
use bittorrent_rs::upload::seed;

pub async fn start() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .without_time()
        .with_target(false)
        .init();

    let args = Args::get();

    let torrent = Torrent::load(args.path)?;
    let storage = Storage::create(&args.output, &torrent.meta.info).await?;
    let client = Client::build()?;
    let session = Arc::new(Session::new(torrent, client.id, storage));
    let torrent = &session.torrent;
    let info = &torrent.meta.info;

    let tracker_response = client
        .send_tracker_req(torrent, session.stats(Some(Event::Started)))
        .await?;
    println!(
        "Downloading {} from {} peers",
        info.name,
        tracker_response.peers.len()
    );

    let progress = ProgressBar::new(info.length).with_style(ProgressStyle::with_template(
        "{bar:40} {bytes}/{total_bytes} {bytes_per_sec} eta {eta}",
    )?);
    let peers = tracker_response.peers.into_iter().map(SocketAddr::V4);
    let downloading = download(Arc::clone(&session), peers, |len| progress.inc(len));
    let seeding = seed(Arc::clone(&session), &client.lst);
    tokio::pin!(seeding);

    tokio::select! {
        result = &mut seeding => result?,
        result = downloading => {
            result?;
            progress.finish();
            announce(&client, &session, Event::Completed).await;
            info!("Download complete, seeding until Ctrl+C");

            tokio::select! {
                result = seeding => result?,
                result = signal::ctrl_c() => result?,
            }
        }
        result = signal::ctrl_c() => result?,
    }

    announce(&client, &session, Event::Stopped).await;

    Ok(())
}

async fn announce(client: &Client, session: &Session, event: Event) {
    if let Err(e) = client
        .send_tracker_req(&session.torrent, session.stats(Some(event)))
        .await
    {
        warn!("Failed to announce {}: {e:#}", event.as_str());
    }
}
