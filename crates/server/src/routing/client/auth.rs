use diesel::prelude::*;
use salvo::oapi::extract::{PathParam, QueryParam};
use salvo::prelude::*;

use crate::core::client::uiaa::AuthType;
use crate::core::identifiers::{OwnedDeviceId, OwnedUserId};
use crate::core::serde::JsonValue;
use crate::data::{connect, schema::user_uiaa_datas};
use crate::{AuthArgs, MatrixError, config};

pub fn authed_router() -> Router {
    Router::with_path("auth/{auth_type}/fallback/web").get(uiaa_fallback)
}

/// Get UIAA fallback web page.
///
/// This endpoint provides a fallback authentication page for clients that
/// don't support a particular authentication type. The page handles the
/// auth stage and notifies the client via postMessage when complete.
#[endpoint]
async fn uiaa_fallback(
    _aa: AuthArgs,
    auth_type: PathParam<AuthType>,
    session: QueryParam<String, true>,
    complete: QueryParam<bool, false>,
    accepted: QueryParam<bool, false>,
    res: &mut Response,
) -> Result<(), crate::AppError> {
    let auth_type = auth_type.into_inner();
    let session_id = session.into_inner();
    let server_name = config::get().server_name.as_str();
    let complete = complete.into_inner().unwrap_or(false);
    let accepted = accepted.into_inner().unwrap_or(false);

    match auth_type {
        AuthType::Dummy | AuthType::Terms => {}
        _ => {
            return Err(MatrixError::unrecognized(format!(
                "Fallback not available for auth type: {}",
                auth_type.as_str()
            ))
            .into());
        }
    }

    let should_complete = match auth_type {
        AuthType::Dummy => complete,
        AuthType::Terms => accepted,
        _ => false,
    };

    if should_complete {
        complete_uiaa_stage(&session_id, auth_type.clone())?;
        let html = render_completion_html(server_name);
        res.add_header("Content-Type", "text/html; charset=utf-8", true)?;
        res.write_body(html)?;
        return Ok(());
    }

    // Generate HTML page based on auth type
    let html = match auth_type {
        AuthType::Dummy => {
            // For m.login.dummy, just show a simple confirmation page
            render_dummy_fallback_html(server_name, &session_id)
        }
        AuthType::Terms => {
            // Terms acceptance fallback
            render_terms_fallback_html(server_name, &session_id)
        }
        _ => unreachable!("auth type checked above"),
    };

    res.add_header("Content-Type", "text/html; charset=utf-8", true)?;
    res.write_body(html)?;
    Ok(())
}

fn load_uiaa_info_by_session(
    session: &str,
) -> Result<(OwnedUserId, OwnedDeviceId, crate::core::client::uiaa::UiaaInfo), crate::AppError> {
    let record = user_uiaa_datas::table
        .filter(user_uiaa_datas::session.eq(session))
        .select((
            user_uiaa_datas::user_id,
            user_uiaa_datas::device_id,
            user_uiaa_datas::uiaa_info,
        ))
        .first::<(OwnedUserId, OwnedDeviceId, JsonValue)>(&mut connect()?)
        .optional()?;
    let Some((user_id, device_id, uiaa_info)) = record else {
        return Err(MatrixError::invalid_param("Invalid session").into());
    };
    let uiaa_info = serde_json::from_value(uiaa_info)?;
    Ok((user_id, device_id, uiaa_info))
}

fn complete_uiaa_stage(session: &str, stage: AuthType) -> Result<(), crate::AppError> {
    let (user_id, device_id, mut uiaa_info) = load_uiaa_info_by_session(session)?;

    if !uiaa_info.completed.contains(&stage) {
        uiaa_info.completed.push(stage);
    }

    let mut completed = false;
    'flows: for flow in &uiaa_info.flows {
        for stage in &flow.stages {
            if !uiaa_info.completed.contains(stage) {
                continue 'flows;
            }
        }
        completed = true;
    }

    if completed {
        crate::uiaa::update_session(&user_id, &device_id, session, None)?;
    } else {
        crate::uiaa::update_session(&user_id, &device_id, session, Some(&uiaa_info))?;
    }

    Ok(())
}

fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

fn escape_js_string(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '<' => out.push_str("\\x3c"),
            '>' => out.push_str("\\x3e"),
            '&' => out.push_str("\\x26"),
            _ => out.push(c),
        }
    }
    out
}

fn url_encode_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{:02X}", b));
        }
    }
    out
}

fn render_completion_html(server_name: &str) -> String {
    let server_name = escape_html(server_name);
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Authentication - {server_name}</title>
    <meta charset="UTF-8">
    <style>
        body {{ font-family: sans-serif; margin: 40px; text-align: center; }}
        .container {{ max-width: 400px; margin: 0 auto; }}
    </style>
</head>
<body>
    <div class="container">
        <h1>Authentication complete</h1>
        <p>You may close this window.</p>
    </div>
    <script>
        if (window.opener) {{
            window.opener.postMessage('authDone', '*');
            window.close();
        }}
    </script>
</body>
</html>"#
    )
}

fn render_dummy_fallback_html(server_name: &str, session_id: &str) -> String {
    let server_name = escape_html(server_name);
    let session_param = url_encode_component(session_id);
    let complete_url = format!(
        "/_matrix/client/v3/auth/m.login.dummy/fallback/web?session={session_param}&complete=true"
    );
    let complete_url = escape_js_string(&complete_url);
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Authentication - {server_name}</title>
    <meta charset="UTF-8">
    <style>
        body {{ font-family: sans-serif; margin: 40px; text-align: center; }}
        .container {{ max-width: 400px; margin: 0 auto; }}
        button {{ padding: 10px 20px; font-size: 16px; cursor: pointer; }}
    </style>
</head>
<body>
    <div class="container">
        <h1>Complete Authentication</h1>
        <p>Click the button below to complete the authentication process.</p>
        <button onclick="complete()">Continue</button>
    </div>
    <script>
        function complete() {{
            window.location.href = '{complete_url}';
        }}
    </script>
</body>
</html>"#
    )
}

fn render_terms_fallback_html(server_name: &str, session_id: &str) -> String {
    let server_name = escape_html(server_name);
    let session_param = url_encode_component(session_id);
    let accept_url = format!(
        "/_matrix/client/v3/auth/m.login.terms/fallback/web?session={session_param}&accepted=true"
    );
    let accept_url = escape_js_string(&accept_url);
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Terms of Service - {server_name}</title>
    <meta charset="UTF-8">
    <style>
        body {{ font-family: sans-serif; margin: 40px; text-align: center; }}
        .container {{ max-width: 600px; margin: 0 auto; }}
        button {{ padding: 10px 20px; font-size: 16px; cursor: pointer; margin: 5px; }}
        .terms {{ text-align: left; border: 1px solid #ccc; padding: 20px; margin: 20px 0; max-height: 300px; overflow-y: auto; }}
    </style>
</head>
<body>
    <div class="container">
        <h1>Terms of Service</h1>
        <div class="terms">
            <p>By using this service, you agree to our terms and conditions.</p>
            <p>Please read the terms carefully before proceeding.</p>
        </div>
        <button onclick="accept()">I Accept</button>
        <button onclick="decline()">Decline</button>
    </div>
    <script>
        function accept() {{
            window.location.href = '{accept_url}';
        }}
        function decline() {{
            if (window.opener) {{
                window.close();
            }}
        }}
    </script>
</body>
</html>"#
    )
}
