use std::fmt::Write;
use std::path::PathBuf;

use crate::admin::Context;
use crate::{AppError, AppResult, config, info};

// pub(super) async fn uptime(ctx: &Context<'_>) -> AppResult<()> {
//     // TODO: admin
//     // let elapsed = self.services.server.started.elapsed().expect("standard duration");

//     // let result = time::pretty(elapsed);
//     // ctx.write_str(&format!("{result}.")).await
//     Ok(())
// }

pub(super) async fn show_config(ctx: &Context<'_>) -> AppResult<()> {
    ctx.write_str(&format!("{}", config::get())).await
}

pub(super) async fn reload_config(ctx: &Context<'_>, _path: Option<PathBuf>) -> AppResult<()> {
    // TODO: admin
    // let path = path.as_deref();
    // config::reload(path)?;

    ctx.write_str("Successfully reconfigured.").await
}

pub(super) async fn list_features(
    ctx: &Context<'_>,
    available: bool,
    enabled: bool,
    comma: bool,
) -> AppResult<()> {
    let delim = if comma { "," } else { " " };
    if enabled && !available {
        let features = info::rustc::features().join(delim);
        let out = format!("`\n{features}\n`");
        return ctx.write_str(&out).await;
    }

    if available && !enabled {
        let features = info::cargo::features().join(delim);
        let out = format!("`\n{features}\n`");
        return ctx.write_str(&out).await;
    }

    let mut features = String::new();
    let enabled = info::rustc::features();
    let available = info::cargo::features();
    for feature in available {
        let active = enabled.contains(&feature.as_str());
        let emoji = if active { "✅" } else { "❌" };
        let remark = if active { "[enabled]" } else { "" };
        writeln!(features, "{emoji} {feature} {remark}")?;
    }

    ctx.write_str(&features).await
}

// pub(super) async fn clear_caches(ctx: &Context<'_>) -> AppResult<()> {
// 	clear_cache(ctx).await;

// 	ctx.write_str("Done.").await
// }

pub(super) async fn admin_notice(ctx: &Context<'_>, message: Vec<String>) -> AppResult<()> {
    let message = message.join(" ");
    crate::admin::send_text(&message).await?;

    ctx.write_str("Notice was sent to #admins").await
}

pub(super) async fn reload_mods(ctx: &Context<'_>) -> AppResult<()> {
    // TODO: reload mods

    ctx.write_str("(TODO) Reloading server...").await
}

pub(super) async fn restart(ctx: &Context<'_>, force: bool) -> AppResult<()> {
    use crate::utils::sys::current_exe_deleted;

    if !force && current_exe_deleted() {
        return Err(AppError::public(
            "The server cannot be restarted because the executable changed. If this is expected \
			 use --force to override.",
        ));
    }

    // TODO: restart server

    ctx.write_str("(TODO) Restarting server...").await
}

pub(super) async fn shutdown(ctx: &Context<'_>) -> AppResult<()> {
    warn!("shutdown command");
    // TODO: shutdown server

    ctx.write_str("(TODO) Shutting down server...").await
}
