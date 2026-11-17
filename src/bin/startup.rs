use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use std::sync::Arc;
use tokio::signal;
use tokio::sync::mpsc;
use tracing::info;
use tracing_subscriber::EnvFilter;

use bittorrent_rs::args_parser::Args;
use bittorrent_rs::client::Client;
use bittorrent_rs::download::download;
use bittorrent_rs::session::Session;
use bittorrent_rs::storage::Storage;
use bittorrent_rs::torrent::Torrent;
use bittorrent_rs::tracker::{Event, announce_all};
use bittorrent_rs::upload::seed;

const PEER_QUEUE: usize = 256;

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
    let port = client.port()?;
    let session = Arc::new(Session::new(torrent, client.id, storage));
    let info = &session.torrent.meta.info;

    let resumed = session.resume().await?;
    if resumed > 0 {
        info!("Resumed {resumed}/{} pieces from disk", info.piece_count());
    }
    let already_complete = session.is_complete();

    let progress = ProgressBar::new(info.length)
        .with_position(session.verified_bytes())
        .with_style(ProgressStyle::with_template(
            "{bar:40} {bytes}/{total_bytes} {bytes_per_sec} eta {eta}",
        )?);

    let (peers_sender, peers) = mpsc::channel(PEER_QUEUE);
    let announcing = session.run_announcer(port, peers_sender);
    let downloading = download(Arc::clone(&session), peers, |len| progress.inc(len));
    let seeding = seed(Arc::clone(&session), &client.lst);
    tokio::pin!(announcing, seeding);

    tokio::select! {
        () = &mut announcing => {}
        result = &mut seeding => result?,
        result = downloading => {
            result?;
            progress.finish();
            if !already_complete {
                announce(&session, port, Event::Completed).await;
            }
            info!("Download complete, seeding until Ctrl+C");

            tokio::select! {
                () = announcing => {}
                result = seeding => result?,
                result = signal::ctrl_c() => result?,
            }
        }
        result = signal::ctrl_c() => result?,
    }

    announce(&session, port, Event::Stopped).await;

    Ok(())
}

async fn announce(session: &Session, port: u16, event: Event) {
    announce_all(
        &session.torrent.meta.trackers,
        session.announce(port, Some(event)),
    )
    .await;
}
