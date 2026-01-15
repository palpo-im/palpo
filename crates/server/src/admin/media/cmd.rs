use std::time::Duration;

use crate::admin::Context;
use crate::admin::utils::parse_local_user_id;
use crate::core::{Mxc, OwnedEventId, OwnedMxcUri, OwnedServerName};
use crate::media::Dimension;
use crate::room::timeline;
use crate::utils::time::parse_timepoint_ago;
use crate::{AppError, AppResult, IsRemoteOrLocal, config, data};

pub(super) async fn delete_media(
    ctx: &Context<'_>,
    mxc: Option<OwnedMxcUri>,
    event_id: Option<OwnedEventId>,
) -> AppResult<()> {
    if event_id.is_some() && mxc.is_some() {
        return Err(AppError::public(
            "Please specify either an MXC or an event ID, not both.",
        ));
    }

    if let Some(mxc) = mxc {
        trace!("Got MXC URL: {mxc}");
        data::media::delete_media(mxc.server_name()?, mxc.media_id()?)?;

        return Err(AppError::public(
            "Deleted the MXC from our database and on our filesystem.",
        ));
    }

    if let Some(event_id) = event_id {
        trace!("Got event ID to delete media from: {event_id}");

        let mut mxc_urls = Vec::with_capacity(4);

        // parsing the PDU for any MXC URLs begins here
        match timeline::get_pdu_json(&event_id) {
            Ok(Some(event_json)) => {
                if let Some(content_key) = event_json.get("content") {
                    debug!("Event ID has \"content\".");
                    let content_obj = content_key.as_object();

                    if let Some(content) = content_obj {
                        // 1. attempts to parse the "url" key
                        debug!("Attempting to go into \"url\" key for main media file");
                        if let Some(url) = content.get("url") {
                            debug!("Got a URL in the event ID {event_id}: {url}");

                            if url.to_string().starts_with("\"mxc://") {
                                debug!("Pushing URL {url} to list of MXCs to delete");
                                let final_url = url.to_string().replace('"', "");
                                mxc_urls.push(final_url);
                            } else {
                                info!(
                                    "Found a URL in the event ID {event_id} but did not start \
									 with mxc://, ignoring"
                                );
                            }
                        }

                        // 2. attempts to parse the "info" key
                        debug!("Attempting to go into \"info\" key for thumbnails");
                        if let Some(info_key) = content.get("info") {
                            debug!("Event ID has \"info\".");
                            let info_obj = info_key.as_object();

                            if let Some(info) = info_obj {
                                if let Some(thumbnail_url) = info.get("thumbnail_url") {
                                    debug!("Found a thumbnail_url in info key: {thumbnail_url}");

                                    if thumbnail_url.to_string().starts_with("\"mxc://") {
                                        debug!(
                                            "Pushing thumbnail URL {thumbnail_url} to list of \
											 MXCs to delete"
                                        );
                                        let final_thumbnail_url =
                                            thumbnail_url.to_string().replace('"', "");
                                        mxc_urls.push(final_thumbnail_url);
                                    } else {
                                        info!(
                                            "Found a thumbnail URL in the event ID {event_id} \
											 but did not start with mxc://, ignoring"
                                        );
                                    }
                                } else {
                                    info!(
                                        "No \"thumbnail_url\" key in \"info\" key, assuming no \
										 thumbnails."
                                    );
                                }
                            }
                        }

                        // 3. attempts to parse the "file" key
                        debug!("Attempting to go into \"file\" key");
                        if let Some(file_key) = content.get("file") {
                            debug!("Event ID has \"file\".");
                            let file_obj = file_key.as_object();

                            if let Some(file) = file_obj {
                                if let Some(url) = file.get("url") {
                                    debug!("Found url in file key: {url}");

                                    if url.to_string().starts_with("\"mxc://") {
                                        debug!("Pushing URL {url} to list of MXCs to delete");
                                        let final_url = url.to_string().replace('"', "");
                                        mxc_urls.push(final_url);
                                    } else {
                                        warn!(
                                            "Found a URL in the event ID {event_id} but did not \
											 start with mxc://, ignoring"
                                        );
                                    }
                                } else {
                                    error!("No \"url\" key in \"file\" key.");
                                }
                            }
                        }
                    } else {
                        return Err(AppError::public(
                            "Event ID does not have a \"content\" key or failed parsing the \
							 event ID JSON.",
                        ));
                    }
                } else {
                    return Err(AppError::public(
                        "Event ID does not have a \"content\" key, this is not a message or an \
						 event type that contains media.",
                    ));
                }
            }
            _ => {
                return Err(AppError::public(
                    "Event ID does not exist or is not known to us.",
                ));
            }
        }

        if mxc_urls.is_empty() {
            return Err(AppError::public("Parsed event ID but found no MXC URLs."));
        }

        let mut mxc_deletion_count: usize = 0;

        for mxc_url in mxc_urls {
            let Mxc {
                server_name,
                media_id,
            } = mxc_url.as_str().try_into()?;
            match data::media::delete_media(server_name, media_id) {
                Ok(()) => {
                    info!("Successfully deleted {mxc_url} from filesystem and database");
                    mxc_deletion_count = mxc_deletion_count.saturating_add(1);
                }
                Err(e) => {
                    warn!("Failed to delete {mxc_url}, ignoring error and skipping: {e}");
                    continue;
                }
            }
        }

        return ctx
            .write_str(&format!(
                "Deleted {mxc_deletion_count} total MXCs from our database and the filesystem \
				 from event ID {event_id}."
            ))
            .await;
    }

    Err(AppError::public(
        "Please specify either an MXC using --mxc or an event ID using --event-id of the \
		 message containing an image. See --help for details.",
    ))
}

pub(super) async fn delete_media_list(ctx: &Context<'_>) -> AppResult<()> {
    if ctx.body.len() < 2
        || !ctx.body[0].trim().starts_with("```")
        || ctx.body.last().unwrap_or(&"").trim() != "```"
    {
        return Err(AppError::public(
            "Expected code block in command body. Add --help for details.",
        ));
    }

    let mut failed_parsed_mxcs: usize = 0;

    let mxc_list = ctx
        .body
        .to_vec()
        .drain(1..ctx.body.len().checked_sub(1).unwrap())
        .filter_map(|mxc_s| {
            mxc_s
                .try_into()
                .inspect_err(|e| {
                    warn!("Failed to parse user-provided MXC URI: {e}");
                    failed_parsed_mxcs = failed_parsed_mxcs.saturating_add(1);
                })
                .ok()
        })
        .collect::<Vec<Mxc<'_>>>();

    let mut mxc_deletion_count: usize = 0;

    for mxc in &mxc_list {
        trace!(%failed_parsed_mxcs, %mxc_deletion_count, "Deleting MXC {mxc} in bulk");
        match data::media::delete_media(mxc.server_name, mxc.media_id) {
            Ok(()) => {
                info!("Successfully deleted {mxc} from filesystem and database");
                mxc_deletion_count = mxc_deletion_count.saturating_add(1);
            }
            Err(e) => {
                warn!("Failed to delete {mxc}, ignoring error and skipping: {e}");
                continue;
            }
        }
    }

    ctx.write_str(&format!(
        "Finished bulk MXC deletion, deleted {mxc_deletion_count} total MXCs from our database \
		 and the filesystem. {failed_parsed_mxcs} MXCs failed to be parsed from the database.",
    ))
    .await
}

pub(super) async fn delete_past_remote_media(
    ctx: &Context<'_>,
    duration: String,
    before: bool,
    after: bool,
    yes_i_want_to_delete_local_media: bool,
) -> AppResult<()> {
    if before && after {
        return Err(AppError::public(
            "Please only pick one argument, --before or --after.",
        ));
    }
    assert!(
        !(before && after),
        "--before and --after should not be specified together"
    );

    let duration = parse_timepoint_ago(&duration)?;
    let deleted_count = crate::media::delete_past_remote_media(
        duration,
        before,
        after,
        yes_i_want_to_delete_local_media,
    )
    .await?;

    ctx.write_str(&format!("Deleted {deleted_count} total files.",))
        .await
}

pub(super) async fn delete_all_media_from_user(
    ctx: &Context<'_>,
    username: String,
) -> AppResult<()> {
    let user_id = parse_local_user_id(&username)?;

    let deleted_count = crate::user::delete_all_media(&user_id).await?;

    ctx.write_str(&format!("Deleted {deleted_count} total files.",))
        .await
}

pub(super) async fn delete_all_media_from_server(
    ctx: &Context<'_>,
    server_name: OwnedServerName,
    yes_i_want_to_delete_local_media: bool,
) -> AppResult<()> {
    if server_name == config::server_name() && !yes_i_want_to_delete_local_media {
        return Err(AppError::public(
            "This command only works for remote media by default.",
        ));
    }

    let Ok(all_mxcs) = crate::media::get_all_mxcs()
        .inspect_err(|e| error!("Failed to get MXC URIs from our database: {e}"))
    else {
        return Err(AppError::public("Failed to get MXC URIs from our database"));
    };

    let mut deleted_count: usize = 0;

    for mxc in all_mxcs {
        let Ok(mxc_server_name) = mxc.server_name().inspect_err(|e| {
            warn!(
                "Failed to parse MXC {mxc} server name from database, ignoring error and \
				 skipping: {e}"
            );
        }) else {
            continue;
        };

        if mxc_server_name != server_name
            || (mxc_server_name.is_local() && !yes_i_want_to_delete_local_media)
        {
            trace!("skipping MXC URI {mxc}");
            continue;
        }

        let mxc: Mxc<'_> = mxc.as_str().try_into()?;

        match crate::media::delete_media(mxc.server_name, mxc.media_id).await {
            Ok(()) => {
                deleted_count = deleted_count.saturating_add(1);
            }
            Err(e) => {
                warn!("Failed to delete {mxc}, ignoring error and skipping: {e}");
                continue;
            }
        }
    }

    ctx.write_str(&format!("Deleted {deleted_count} total files.",))
        .await
}

pub(super) async fn get_file_info(ctx: &Context<'_>, mxc: OwnedMxcUri) -> AppResult<()> {
    let Mxc {
        server_name,
        media_id,
    } = mxc.as_str().try_into()?;
    let metadata = data::media::get_metadata(server_name, media_id)?;

    ctx.write_str(&format!("```\n{metadata:#?}\n```")).await
}

// pub(super) async fn fetch_remote_file(
//     ctx: &Context<'_>,
//     mxc: OwnedMxcUri,
//     server: Option<OwnedServerName>,
//     timeout: u32,
// ) -> AppResult<()> {
//     let mxc: Mxc<'_> = mxc.as_str().try_into()?;
//     let timeout = Duration::from_millis(timeout.into());
//     let mut result = crate::media::fetch_remote_content(&mxc, &mxc.server_name, &mxc.media_id)
//         .await?;

//     // Grab the length of the content before clearing it to not flood the output
//     let len = result.content.as_ref().expect("content").len();
//     result.content.as_mut().expect("content").clear();

//     ctx.write_str(&format!(
//         "```\n{result:#?}\nreceived {len} bytes for file content.\n```"
//     ))
//     .await
// }

pub(super) async fn fetch_remote_thumbnail(
    ctx: &Context<'_>,
    mxc: OwnedMxcUri,
    server: Option<OwnedServerName>,
    timeout: u32,
    width: u32,
    height: u32,
) -> AppResult<()> {
    let mxc: Mxc<'_> = mxc.as_str().try_into()?;
    let timeout = Duration::from_millis(timeout.into());
    let dim = Dimension::new(width, height, None);
    let mut result =
        crate::media::fetch_remote_thumbnail(&mxc, None, server.as_deref(), timeout, &dim).await?;

    // Grab the length of the content before clearing it to not flood the output
    let len = result.content.as_ref().expect("content").len();
    result.content.as_mut().expect("content").clear();

    ctx.write_str(&format!(
        "```\n{result:#?}\nreceived {len} bytes for file content.\n```"
    ))
    .await
}
