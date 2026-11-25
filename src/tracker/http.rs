use anyhow::Result;
use percent_encoding::{NON_ALPHANUMERIC, percent_encode};
use reqwest::Client;
use std::fmt::Write;
use url::Url;

use crate::bencode::BencodeParser;
use crate::tracker::{Announce, TrackerResponse};

const NUM_WANT: u32 = 100;

pub async fn announce(
    client: &Client,
    mut url: Url,
    request: &Announce,
) -> Result<TrackerResponse> {
    let mut query = url.query().map(str::to_owned).unwrap_or_default();
    if !query.is_empty() {
        query.push('&');
    }

    write!(
        query,
        "info_hash={}&peer_id={}&port={}&uploaded={}&downloaded={}&left={}&compact=1&numwant={NUM_WANT}",
        percent_encode(request.info_hash.as_ref(), NON_ALPHANUMERIC),
        percent_encode(request.peer_id.as_ref(), NON_ALPHANUMERIC),
        request.port,
        request.uploaded,
        request.downloaded,
        request.left,
    )?;
    if let Some(event) = request.event {
        write!(query, "&event={}", event.as_str())?;
    }
    url.set_query(Some(&query));

    let body = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    BencodeParser::new(body.to_vec()).parse()?.try_into()
}
